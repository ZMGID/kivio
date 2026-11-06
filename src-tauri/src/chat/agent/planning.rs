use super::content::{ContentPhase, ContentKind, ContentBlock};
use crate::chat::model::{
    stream_with_chat_provider, FinishKind, GenerateOptions, GenerateOutput, GenerateRequest,
    ModelError, ModelErrorKind, PendingToolCall, StreamPart, StreamSink,
};

use crate::mcp::ChatToolDefinition;

use super::finalize::{
    cancelled_tool_round_run_result, tool_planning_failed_run_result, RunResultBuilder,
};
use super::loop_::{LoopEnv, RunState};
use super::rounds::visible_tool_segment_calls;
use super::stop::sanitize_assistant_text_response;
use super::stream::{
    normalize_stream_output, validate_stream_output, AgentStreamSink, ToolCallDraftTracker,
    WebSearchCardTracker,
};
use super::types::{AgentRunResult, AgentStreamPolicy};

pub(crate) struct PlannedToolRound {
    pub(crate) message: crate::chat::model::ModelMessage,
    pub(crate) finish_kind: Option<FinishKind>,
    pub(crate) tool_calls: Vec<PendingToolCall>,
    pub(crate) drafts: ToolCallDraftTracker,
}

pub(crate) enum PlanningStepOutcome {
    FinalAnswer(GenerateOutput),
    /// Planning produced tool calls; the skeleton hands them to `run_tool_round`.
    ToolCalls(PlannedToolRound),
    /// `state.tools` was narrowed to skill-native tools; the skeleton retries.
    RetryWithSkillTools,
    /// Planning returned an empty assistant response (no text, no tool calls) —
    /// flaky gateways do this intermittently. Retried once via the skeleton's
    /// `continue`; a second empty lands in FinalAnswer and fails at finalize
    /// with the existing "empty assistant response" error.
    RetryEmptyResponse,
    /// Provider rejected tools; `state.provider_tools_unsupported` was set and a
    /// step was pushed. The skeleton breaks out of the tool loop.
    ToolsUnsupported,
    /// Tool-call argument drafting failed mid-stream; the run ends immediately.
    DraftFailed(AgentRunResult),
    /// A later-round planning call hard-failed but tool results already exist;
    /// the run ends with those gathered results instead of bubbling an error
    /// (统一恢复:不空手而归).
    Recovered(AgentRunResult),
    /// Streaming was cancelled after partial plain-text output (no tool drafts
    /// started); the run ends immediately preserving the generated text. The
    /// stream layer already emitted the single done("cancelled") event.
    Cancelled(AgentRunResult),
}

pub(crate) async fn planning_step(
    env: &LoopEnv<'_>,
    state: &mut RunState,
    round: u32,
) -> Result<PlanningStepOutcome, String> {
    let config = env.config;
    let host = env.host;
    let step_number = state.step_number;
    // 循环内上下文治理：超限时先 snip / 摘要，得到本步发送视图（未超限时为原样 clone）。
    let active_tools = state.tools.clone();
    let mut send_messages =
        super::compaction::maybe_compact_send_view(env, state, &active_tools).await?;

    // ZCode rapid-refill breaker: the context refilled right after compaction several
    // times in a row (a file or tool output is too large), so compacting again cannot
    // help. End the turn with the gathered tool results via recovery's degrade path.
    // Failed compaction also stops a request known to exceed the model input allowance.
    if state.compaction_blocked {
        eprintln!(
            "Chat context compaction: context cannot fit safely after compaction; ending turn with gathered results"
        );
        let kind = crate::chat::agent::recovery::FailureKind::ContextOverflow;
        let content = crate::chat::agent::recovery::assemble_results_from_tool_records(
            &state.tool_records,
            &config.language,
            kind,
        );
        // 有可兜底素材 → 用降级摘要收尾；没有素材（content 为空）→ 退回去敏/超长静态文案，
        // 但仍以「已收尾」结束本轮，绝不再循环触发压缩失败。
        let content = if content.trim().is_empty() {
            crate::chat::agent::recovery::overflow_static_message(&config.language)
        } else {
            content
        };
        // 为降级文案预约一个文本 segment，让它在 transcript 里正常渲染（与其它收尾路径一致）。
        let degrade_segment = state.segment_builder.reserve(
            ContentKind::Text,
            ContentPhase::Plain,
            Some(step_number),
            Some(round),
            &format!("step_{step_number}_compaction_blocked"),
        );
        return Ok(PlanningStepOutcome::Recovered(
            RunResultBuilder::new(host, content)
                .segment(&degrade_segment)
                .emit_done("done")
                .outcome(super::types::AgentRunStatus::CompactionBlocked)
                .finish(
                    std::mem::take(&mut state.segment_builder),
                    &state.planning_reasoning_parts,
                    std::mem::take(&mut state.tool_records),
                    std::mem::take(&mut state.generated_api_messages),
                ),
        ));
    }

    let stream_policy = AgentStreamPolicy::PlanningNoDoneUntilNoTools;
    let planning_reasoning_segment = state.segment_builder.reserve(
        ContentKind::Reasoning,
        ContentPhase::ToolLoop,
        Some(step_number),
        Some(round),
        &format!("step_{step_number}_reasoning"),
    );
    // 在 reasoning 与正文之间预留内置搜索实时卡的 order 槽（任务 07-23）：搜索发生时卡插到
    // 这里 → 渲染在答案文本之前；未搜索则该 order 留空洞无害。
    let web_search_order = state.segment_builder.reserve_order();
    let planning_text_segment = state.segment_builder.reserve(
        ContentKind::Text,
        ContentPhase::ToolLoop,
        Some(step_number),
        Some(round),
        &format!("step_{step_number}_text"),
    );
    let planning_tool_drafts = ToolCallDraftTracker::new(
        active_tools.clone(),
        round,
        Some(step_number),
        state.segment_builder.next_order(),
    );
    planning_tool_drafts.enable_speculation(
        host,
        &config.settings,
        &super::execute::ToolExecutionContext {
            conversation_id: &config.conversation_id,
            run_id: &config.run_id,
            message_id: &config.message_id,
            provider: &config.provider,
            model: &config.model,
            generation: config.generation,
            round,
            depth: config.depth,
            tool_conversation_id: &config.tool_conversation_id,
            tool_call_id: "",
            plan_state: &config.plan_state,
            mcp_tool_ids: &config.mcp_tool_ids,
            mcp_server_ids: &config.mcp_server_ids,
            approval_policy: &config.effective_chat_tools.approval_policy,
            interactive_host: config.interactive_host,
        },
    );
    // 内置搜索实时卡追踪器：仅在会话 Builtin 模式且模型支持时建（否则 None，走现状路径）。
    let planning_web_search_tracker = if config.builtin_web_search_active() {
        Some(WebSearchCardTracker::new(
            web_search_order,
            Some(round),
            ContentPhase::ToolLoop,
            config.provider.name.clone(),
        ))
    } else {
        None
    };
    // 内置搜索由实时卡追踪器边流边合成（take_card 落 Success 终态卡）。
    let mut overflow_attempted = false;
    let planning_result = loop {
        match super::recovery::stream_with_truncation_recovery(
            env,
            state,
            super::provider_runtime::StreamRequest {
                host,
                provider: &config.provider,
                model: &config.model,
                messages: send_messages.clone(),
                tools: Some(&active_tools),
                tool_choice: crate::chat::model::ToolChoice::Auto,
                retry_attempts: config.retry_attempts,
                thinking_enabled: config.thinking_enabled,
                thinking_level: config.thinking_level.clone(),
                builtin_web_search: config.builtin_web_search_active(),
                max_output_tokens: config.max_output_tokens,
                conversation_id: &config.conversation_id,
                message_id: &config.message_id,
                label: "Chat tools planning",
                policy: stream_policy,
                text_segment: Some(planning_text_segment.clone()),
                reasoning_segment: Some(planning_reasoning_segment.clone()),
                tool_draft_tracker: Some(planning_tool_drafts.clone()),
                web_search_tracker: planning_web_search_tracker.clone(),
            },
        )
        .await
        {
            Ok(mut stream) => {
                if stream.cancelled {
                    let partial = sanitize_assistant_text_response(&stream.text);
                    if partial.trim().is_empty() || planning_tool_drafts.has_started() {
                        // No preservable partial answer (empty text, or tool-arg
                        // drafting had started and is now incomplete). The stream
                        // layer already emitted the single done("cancelled") event,
                        // so build the cancelled result WITHOUT re-emitting — but
                        // still end with Ok(cancelled_result) carrying the rounds
                        // already accumulated, so the turn is persisted instead of
                        // dropped via a bare Err("cancelled").
                        return Ok(PlanningStepOutcome::Cancelled(
                            cancelled_tool_round_run_result(
                                &state.planning_reasoning_parts,
                                std::mem::take(&mut state.tool_records),
                                std::mem::take(&mut state.segment_builder).all(),
                                std::mem::take(&mut state.generated_api_messages),
                            ),
                        ));
                    }
                    // Partial plain text was already streamed to the frontend and the
                    // stream layer already emitted the single done("cancelled") event;
                    // preserve the generated text instead of dropping the whole turn.
                    // Append the reasoning segment first (its reserved order is lower
                    // than the text segment's) so the persisted timeline keeps reasoning
                    // above the answer; otherwise normalize_assistant_segments would add
                    // a trailing reasoning segment that renders below the text.
                    if let Some(reasoning_text) = stream
                        .reasoning
                        .as_deref()
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                    {
                        let mut reasoning_segment = planning_reasoning_segment.clone();
                        reasoning_segment.phase = ContentPhase::Plain;
                        state.segment_builder.append_text_from_template(
                            &reasoning_segment,
                            reasoning_text.to_string(),
                        );
                    }
                    let mut segment = planning_text_segment.clone();
                    segment.phase = ContentPhase::Plain;
                    return Ok(PlanningStepOutcome::Cancelled(
                        RunResultBuilder::new(host, partial)
                            .segment(&segment)
                            .api_reasoning(stream.reasoning.clone())
                            .reasoning_tail(stream.reasoning)
                            .outcome(super::types::AgentRunStatus::Cancelled)
                            .finish(
                                std::mem::take(&mut state.segment_builder),
                                &state.planning_reasoning_parts,
                                std::mem::take(&mut state.tool_records),
                                std::mem::take(&mut state.generated_api_messages),
                            ),
                    ));
                }
                state.merge_usage_with_billing(
                    stream.usage.clone(),
                    stream.billing_usage.clone(),
                    env,
                    true,
                );
                state.generated_images.append(&mut stream.images);
                let finish_kind = stream.finish_reason.as_ref().map(|reason| reason.kind);
                break Ok((stream, finish_kind));
            }
            Err(err) if planning_tool_drafts.has_started() => {
                eprintln!(
                    "Chat tools planning failed after stream recovery; surfacing tool draft error: {}",
                    err
                );
                return Ok(PlanningStepOutcome::DraftFailed(
                    tool_planning_failed_run_result(
                        host,
                        config,
                        std::mem::take(&mut state.segment_builder),
                        planning_text_segment.clone(),
                        planning_tool_drafts,
                        &state.planning_reasoning_parts,
                        std::mem::take(&mut state.generated_api_messages),
                        err.to_string(),
                    ),
                ));
            }
            Err(err) if !overflow_attempted && err.kind == ModelErrorKind::ContextOverflow => {
                overflow_attempted = true;
                let previous = send_messages.clone();
                send_messages =
                    super::compaction::compact_send_view(env, state, true, &active_tools).await?;
                if state.compaction_blocked || previous == send_messages {
                    break Err(err);
                }
                continue;
            }
            Err(err) => break Err(err),
        }
    };
    let (message, finish_kind) = match planning_result {
        Ok(planned) => planned,
        Err(err) if err.kind == ModelErrorKind::ToolsUnsupported => {
            let skill_only: Vec<ChatToolDefinition> = state
                .tools
                .iter()
                .filter(|tool| tool.source == "skill")
                .cloned()
                .collect();
            if !state.tried_skill_only_tools
                && skill_only.len() < state.tools.len()
                && !skill_only.is_empty()
            {
                eprintln!(
                    "Chat provider {} rejected tools; retrying with skill-native tools only",
                    config.provider.id
                );
                state.tools = skill_only;
                state.tried_skill_only_tools = true;
                return Ok(PlanningStepOutcome::RetryWithSkillTools);
            }
            eprintln!(
                "Chat provider {} rejected tools; falling back to plain chat",
                config.provider.id
            );
            state.provider_tools_unsupported = true;
            return Ok(PlanningStepOutcome::ToolsUnsupported);
        }
        Err(err) => {
            // 统一恢复:多轮中途 planning 调用硬失败时,若已收集到工具结果,不要让
            // 整轮报错丢弃成果——走与 synthesis 同一条恢复阶梯(去敏重做 → 确定性兜底),
            // 而非直接堆原始结果。
            if !state.tool_records.is_empty() {
                let recovered = super::synthesis::recover_synthesis(env, state, &err).await?;
                let replay = recovered.replay;
                let content = recovered.text;
                if !content.trim().is_empty() {
                    eprintln!("Chat planning call failed mid-run; recovered: {err}");
                    // 降级文案是本轮的**正文**，段 phase 必须是 Synthesis（不能留 ToolLoop）：
                    // content_from_segments 只认 Plain|Synthesis，留 ToolLoop 会让
                    // normalize_assistant_segments 以为正文没落段而再补一条，正文渲染两遍。
                    let mut segment = planning_text_segment.clone();
                    segment.phase = ContentPhase::Synthesis;
                    return Ok(PlanningStepOutcome::Recovered(
                        RunResultBuilder::new(host, content)
                            .segment(&segment)
                            .emit_done("done")
                            .outcome(super::types::AgentRunStatus::Recovered)
                            .replay(replay)
                            .degraded(state.degraded.take())
                            .finish(
                                std::mem::take(&mut state.segment_builder),
                                &state.planning_reasoning_parts,
                                std::mem::take(&mut state.tool_records),
                                std::mem::take(&mut state.generated_api_messages),
                            ),
                    ));
                }
            }
            return Err(err.message);
        }
    };
    // 内置搜索：实时卡追踪器边流边合成（take_card 落 Success 终态卡）。
    if let Some(tracker) = planning_web_search_tracker.as_ref() {
        if let Some(segment) = tracker.take_card() {
            state
                .segment_builder
                .append_existing_segments(vec![segment]);
        }
    }
    let tool_calls = message.tool_calls.clone();
    planning_tool_drafts
        .reconcile_speculation(&tool_calls, finish_kind == Some(FinishKind::Length));
    if tool_calls.is_empty() {
        let response = sanitize_assistant_text_response(&message.text);
        // 空响应重试（一次）：正文空 + 无工具调用 = 抽风网关的典型症状（HTTP 200 但
        // 正文什么都没给，可能残留一段 reasoning）。这种消息走到 finalize 必报
        // "empty assistant response"——与其断轮不如原地重试一次；再空则照旧报错。
        // 例外：本轮出了图（hosted image generation，如 Responses 的 image_generation_call）
        // 时正文本就是空串——图即答案，重试只会再生成一张，必须放行。
        if response.trim().is_empty()
            && state.generated_images.is_empty()
            && !state.planning_empty_retried
        {
            state.planning_empty_retried = true;
            eprintln!("Chat tools planning returned an empty response; retrying once");
            return Ok(PlanningStepOutcome::RetryEmptyResponse);
        }
        if !response.trim().is_empty() {
            let mut segment = planning_text_segment.clone();
            segment.phase = ContentPhase::Plain;
            host.content("", None, Some(&segment));
            state
                .segment_builder
                .append_text_from_template(&segment, response);
        }
        if let Some(reasoning) = message
            .reasoning
            .clone()
            .filter(|text| !text.trim().is_empty())
        {
            let mut segment = planning_reasoning_segment.clone();
            segment.phase = ContentPhase::Plain;
            host.content("", None, Some(&segment));
            state
                .segment_builder
                .append_text_from_template(&segment, reasoning);
        }
        return Ok(PlanningStepOutcome::FinalAnswer(message));
    }
    let planning_text = sanitize_assistant_text_response(&message.text);
    if !planning_text.trim().is_empty() {
        state
            .segment_builder
            .append_text_from_template(&planning_text_segment, planning_text);
    }
    if let Some(reasoning) = message
        .reasoning
        .clone()
        .filter(|text| !text.trim().is_empty())
    {
        state
            .segment_builder
            .append_text_from_template(&planning_reasoning_segment, reasoning.clone());
        state.planning_reasoning_parts.push(reasoning);
    }

    let visible_tool_calls =
        visible_tool_segment_calls(&state.tools, &state.blocked_tool_calls, &tool_calls);
    let draft_tool_segments = planning_tool_drafts.segments();
    if draft_tool_segments.is_empty() {
        let tool_segments = state.segment_builder.append_tool_calls(
            ContentPhase::ToolLoop,
            Some(step_number),
            round,
            &visible_tool_calls,
        );
        for segment in &tool_segments {
            host.content("", None, Some(segment));
        }
    } else {
        state
            .segment_builder
            .append_existing_segments(draft_tool_segments);
    }
    Ok(PlanningStepOutcome::ToolCalls(PlannedToolRound {
        message: message.replay_message(),
        finish_kind,
        tool_calls,
        drafts: planning_tool_drafts,
    }))
}

/// 无头流式收集 sink：丢弃所有增量。摘要调用只消费返回的 `GenerateOutput.text`
/// （所有 provider 适配器都在流式路径上从累积增量填好 `text`），故无需在此累积；
struct DiscardStreamSink;

impl StreamSink for DiscardStreamSink {
    fn emit(&mut self, _: StreamPart) -> Result<(), ModelError> {
        Ok(())
    }
}

/// 走 provider 的**流式** `stream(...)`（经 `send_with_failover` + SSE 累积，与 planning/synthesis
/// 同一路径），用一个丢弃增量的 sink 触发流式，返回 provider 组装好的 `GenerateOutput`。
///
/// 所有适配器（`openai` / `anthropic` / `responses`）都在流式路径上把累积结果填进返回的
/// `GenerateOutput.text`/`reasoning`，故无需 sink 侧再累积兜底。
pub(crate) async fn generate_via_stream_collect(
    state: &crate::state::AppState,
    provider: &crate::settings::ModelProvider,
    retry_attempts: usize,
    request: GenerateRequest,
) -> Result<GenerateOutput, ModelError> {
    let mut sink = DiscardStreamSink;
    stream_with_chat_provider(state, provider, retry_attempts, request, &mut sink).await
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn stream_scoped_chat_completion_inner(
    state: &crate::state::AppState,
    host: &super::context::AgentContext<'_>,
    provider: &crate::settings::ModelProvider,
    model: &str,
    messages: Vec<super::history::RuntimeMessage>,
    tools: Option<&[ChatToolDefinition]>,
    tool_choice: crate::chat::model::ToolChoice,
    retry_attempts: usize,
    thinking_enabled: bool,
    thinking_level: Option<String>,
    builtin_web_search: bool,
    max_output_tokens: u32,
    conversation_id: &str,
    message_id: &str,
    label: &str,
    policy: AgentStreamPolicy,
    text_segment: Option<ContentBlock>,
    reasoning_segment: Option<ContentBlock>,
    tool_draft_tracker: Option<ToolCallDraftTracker>,
    web_search_tracker: Option<WebSearchCardTracker>,
) -> Result<GenerateOutput, ModelError> {
    host.context_prepared(&provider.id, model, &messages, tools.unwrap_or_default());
    let request = super::history::request_from_history(
        &messages,
        model,
        tools.unwrap_or_default().iter().map(Into::into).collect(),
        GenerateOptions {
            thinking_enabled,
            thinking_level,
            builtin_web_search,
            tool_choice,
            max_tokens: max_output_tokens,
            ..GenerateOptions::default()
        },
        crate::chat::model::RequestMetadata {
            label: label.to_string(),
            conversation_id: Some(conversation_id.to_string()),
            message_id: Some(message_id.to_string()),
            ..Default::default()
        },
    );
    crate::api::with_retry_budget(retry_attempts, async {
        let budget = crate::api::RetryBudget::current_or_new(retry_attempts);
        loop {
            let mut sink = AgentStreamSink::new(
                host,
                matches!(policy, AgentStreamPolicy::PlanningNoDoneUntilNoTools),
                text_segment.clone(),
                reasoning_segment.clone(),
                tool_draft_tracker.clone(),
                web_search_tracker.clone(),
            );
            let result = {
                let (progress_tx, mut progress_rx) = tokio::sync::mpsc::unbounded_channel();
                let stream_future = crate::api::RETRY_PROGRESS.scope(
                    progress_tx,
                    stream_with_chat_provider(
                        state,
                        provider,
                        retry_attempts,
                        request.clone(),
                        &mut sink,
                    ),
                );
                tokio::pin!(stream_future);
                loop {
                    tokio::select! {
                        result = &mut stream_future => break Some(result),
                        Some(retry) = progress_rx.recv() => host.retry(super::events::RetryStatus { reason: retry.reason, attempt: retry.attempt, max_retries: retry.max_retries, retry_at_ms: retry.retry_at_ms }),
                        _ = host.control.wait_inactive() => break None,
                    }
                }
            };
            let Some(result) = result.filter(|_| host.control.active()) else {
                let (content, reasoning) = sink.snapshot();
                return Ok(GenerateOutput::cancelled(
                    content.trim().to_string(),
                    (!reasoning.trim().is_empty()).then(|| reasoning.trim().to_string()),
                ));
            };
            let output = match result {
                Ok(output) => output,
                Err(err) if err.is_retryable_stream_error() => {
                    let Some((retry, delay_ms)) = budget.schedule(None, "Stream interrupted", false) else {
                        return Err(err);
                    };
                    sink.discard_attempt();
                    host.retry(super::events::RetryStatus { reason: retry.reason, attempt: retry.attempt, max_retries: retry.max_retries, retry_at_ms: retry.retry_at_ms });
                    let delay = std::time::Duration::from_millis(delay_ms);
                    tokio::select! {
                        _ = tokio::time::sleep(delay) => {},
                        _ = host.control.wait_inactive() => {
                            return Ok(GenerateOutput::cancelled(String::new(), None));
                        }
                    }
                    continue;
                }
                Err(err) => return Err(err),
            };
            host.status(None);
            sink.flush_pending_text();
            // 内置搜索实时卡定稿：流成功结束 ⇒ 翻 Success 并发终态记录（取消路径已在上面的
            // select 分支 return，不会到这里，故取消时实时卡不落 Success，与 draft 行为一致）。
            if let Some(record) = sink.finish_web_search_card() {
                host.search_updated(&record, false);
            }
            let (snapshot_content, snapshot_reasoning) = sink.snapshot();
            let stream_output = normalize_stream_output(
                output,
                snapshot_content,
                snapshot_reasoning,
                policy,
            );
            // Roll back exactly what this sink emitted, including Unicode and
            // provider-specific text normalization, before semantic recovery.
            if super::recovery::is_recoverable_text_length(&stream_output, max_output_tokens) {
                sink.discard_attempt();
            }
            validate_stream_output(label, policy, &stream_output)
                .map_err(|message| ModelError::with_kind(message, ModelErrorKind::EmptyResponse))?;
            return Ok(stream_output);
        }
    }).await
}
