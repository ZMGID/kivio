use super::content::{ContentBlock, ContentKind, ContentPhase};
use crate::model::{FinishKind, GenerateOutput, ModelError, ModelErrorKind, PendingToolCall};

use super::session::{SessionPolicy, ToolsRejected};

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
    pub(crate) message: crate::model::ModelMessage,
    pub(crate) finish_kind: Option<FinishKind>,
    pub(crate) tool_calls: Vec<PendingToolCall>,
    pub(crate) drafts: ToolCallDraftTracker,
}

pub(crate) enum PlanningStepOutcome {
    FinalAnswer(GenerateOutput),
    /// Planning produced tool calls; the skeleton hands them to `run_tool_round`.
    ToolCalls(PlannedToolRound),
    /// `state.tools` was narrowed to skill-native tools; the skeleton retries.
    RetryWithTools,
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
    session: &mut dyn SessionPolicy,
) -> Result<PlanningStepOutcome, String> {
    let config = env.config;
    let host = env.host;
    let step_number = state.step_number;
    // 循环内上下文治理：超限时先 snip / 摘要，得到本步发送视图（未超限时为原样 clone）。
    let active_tools = state.tools.clone();
    let model_tools: Vec<_> = active_tools.iter().map(|tool| tool.model.clone()).collect();
    let mut send_messages =
        super::compaction::maybe_compact_send_view(env, state, &model_tools, session).await?;

    // ZCode rapid-refill breaker: the context refilled right after compaction several
    // times in a row (a file or tool output is too large), so compacting again cannot
    // help. End the turn with the gathered tool results via recovery's degrade path.
    // Failed compaction also stops a request known to exceed the model input allowance.
    if state.compaction_blocked {
        eprintln!(
            "Chat context compaction: context cannot fit safely after compaction; ending turn with gathered results"
        );
        let content = session.context_blocked(super::recovery::snapshot(state));
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
    let execution_context = super::execute::ToolExecutionContext {
        conversation_id: &config.conversation_id,
        run_id: &config.run_id,
        message_id: &config.message_id,
        generation: config.generation,
        round,
        tool_call_id: "",
    };
    // 内置搜索实时卡追踪器：仅在会话 Builtin 模式且模型支持时建（否则 None，走现状路径）。
    let planning_web_search_tracker = if config.builtin_web_search_active() {
        Some(WebSearchCardTracker::new(
            web_search_order,
            Some(round),
            ContentPhase::ToolLoop,
            config.provider_label.clone(),
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
            super::recovery::StreamRequest {
                call: config.model_call(
                    send_messages.clone(),
                    model_tools.clone(),
                    crate::model::ToolChoice::Auto,
                    "Chat tools planning",
                    super::provider_runtime::ModelDelivery::Streamed,
                ),
                policy: stream_policy,
                text_segment: Some(planning_text_segment.clone()),
                reasoning_segment: Some(planning_reasoning_segment.clone()),
                tool_draft_tracker: Some(planning_tool_drafts.clone()),
                web_search_tracker: planning_web_search_tracker.clone(),
                speculation: Some((env.executor, &execution_context)),
            },
            session,
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
                        session.fallback(super::recovery::FallbackReason::FailedPlanning),
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
                    super::compaction::compact_send_view(env, state, true, &model_tools, session)
                        .await?;
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
            match session.tools_rejected(state.snapshot()) {
                ToolsRejected::Retry(tools) => {
                    state.tools = tools;
                    return Ok(PlanningStepOutcome::RetryWithTools);
                }
                ToolsRejected::Plain(prompt) => {
                    state.provider_tools_unsupported = true;
                    state.fallback_prompt = prompt;
                    return Ok(PlanningStepOutcome::ToolsUnsupported);
                }
            }
        }
        Err(err) => {
            // 统一恢复:多轮中途 planning 调用硬失败时,若已收集到工具结果,不要让
            // 整轮报错丢弃成果——走与 synthesis 同一条恢复阶梯(去敏重做 → 确定性兜底),
            // 而非直接堆原始结果。
            if !state.tool_records.is_empty() {
                let recovered =
                    super::synthesis::recover_synthesis(env, state, &err, session).await?;
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

    let visible_tool_calls = visible_tool_segment_calls(
        env.executor,
        &state.tools,
        &state.blocked_tool_calls,
        &tool_calls,
    );
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

#[allow(clippy::too_many_arguments)]
pub(crate) async fn stream_model_call<'a>(
    runtime: &dyn super::provider_runtime::ModelRuntime,
    host: &'a super::context::AgentContext<'a>,
    call: super::provider_runtime::ModelCall,
    policy: AgentStreamPolicy,
    text_segment: Option<ContentBlock>,
    reasoning_segment: Option<ContentBlock>,
    tool_draft_tracker: Option<ToolCallDraftTracker>,
    web_search_tracker: Option<WebSearchCardTracker>,
    speculation: Option<(
        &'a dyn super::execute::ToolExecutor,
        &'a super::execute::ToolExecutionContext<'a>,
    )>,
) -> Result<GenerateOutput, ModelError> {
    let max_output_tokens = call.options.max_tokens;
    let label = call.metadata.label.clone();
    let mut sink = AgentStreamSink::new(
        host,
        matches!(policy, AgentStreamPolicy::PlanningNoDoneUntilNoTools),
        text_segment,
        reasoning_segment,
        tool_draft_tracker,
        web_search_tracker,
    )
    .with_speculation(speculation);
    let result = {
        let generate = runtime.generate(call, &mut sink);
        tokio::pin!(generate);
        tokio::select! {
            result = &mut generate => Some(result),
            _ = host.control.wait_inactive() => None,
        }
    };
    let Some(result) = result.filter(|_| host.control.active()) else {
        let (content, reasoning) = sink.snapshot();
        return Ok(GenerateOutput::cancelled(
            content.trim().to_string(),
            (!reasoning.trim().is_empty()).then(|| reasoning.trim().to_string()),
        ));
    };
    let output = result?;
    if !sink.request_was_started() {
        return Ok(output);
    }
    host.status(None);
    sink.flush_pending_text();
    if let Some(record) = sink.finish_web_search_card() {
        host.search_updated(&record, false);
    }
    let (snapshot_content, snapshot_reasoning) = sink.snapshot();
    let stream_output =
        normalize_stream_output(output, snapshot_content, snapshot_reasoning, policy);
    if super::recovery::is_recoverable_text_length(&stream_output, max_output_tokens) {
        sink.discard_attempt();
    }
    validate_stream_output(&label, policy, &stream_output)
        .map_err(|message| ModelError::with_kind(message, ModelErrorKind::EmptyResponse))?;
    Ok(stream_output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::content::InputKind;
    use crate::agent::context::AgentContext;
    use crate::agent::control::{AgentRunControl, AgentRunHandle, InputAdmission, StopAdmission};
    use crate::agent::events::{AgentEvent, AgentRunIdentity};
    use crate::agent::host::{AgentHost, AgentHostFuture};
    use crate::agent::provider_runtime::{
        ModelCall, ModelDelivery, ModelOutputSink, ModelRuntime, ProviderFuture,
    };
    use crate::agent::steering::SteeringMessage;

    #[tokio::test]
    async fn synthetic_model_completion_does_not_enter_stream_normalization() {
        struct Completed;
        impl AgentHost for Completed {}
        impl AgentRunControl for Completed {
            fn active(&self) -> bool {
                true
            }
            fn stop(&self) -> StopAdmission {
                StopAdmission::Requested
            }
            fn submit(&self, _: InputKind, _: SteeringMessage) -> InputAdmission {
                InputAdmission::Unsupported
            }
            fn take(&self, _: InputKind) -> Vec<SteeringMessage> {
                Vec::new()
            }
            fn wait_inactive(&self) -> AgentHostFuture<'_, ()> {
                Box::pin(std::future::pending())
            }
        }
        impl ModelRuntime for Completed {
            fn generate<'a>(
                &'a self,
                _: ModelCall,
                _: &'a mut dyn ModelOutputSink,
            ) -> ProviderFuture<'a, Result<GenerateOutput, ModelError>> {
                Box::pin(async { Ok(GenerateOutput::text("  completed yield  ".into(), None)) })
            }
        }
        let identity = AgentRunIdentity {
            conversation_id: "conversation",
            run_id: "run",
            message_id: "message",
            generation: 1,
        };
        let events = |_event: &AgentEvent<'_>| -> crate::agent::events::EventDelivery {
            panic!("synthetic completion emits no streaming events")
        };
        let context = AgentContext::new(
            &Completed,
            &events,
            AgentRunHandle::new(identity, &Completed),
        );
        let output = stream_model_call(
            &Completed,
            &context,
            ModelCall {
                provider_id: String::new(),
                model: String::new(),
                history: Vec::new(),
                tools: Vec::new(),
                options: Default::default(),
                metadata: Default::default(),
                delivery: ModelDelivery::Streamed,
            },
            AgentStreamPolicy::PlanningNoDoneUntilNoTools,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(output.text, "  completed yield  ");
        assert!(output.tool_calls.is_empty());
    }
}
