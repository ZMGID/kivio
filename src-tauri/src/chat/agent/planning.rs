use serde_json::Value;

use crate::chat::model::PendingToolCall;
use crate::chat::types::{ChatMessageSegmentKind, ChatMessageSegmentPhase};
use crate::mcp::ChatToolDefinition;

use super::finalize::{
    cancelled_tool_round_run_result, tool_planning_failed_run_result, RunResultBuilder,
};
use super::loop_::{LoopEnv, RunState};
use super::rounds::visible_tool_segment_calls;
use super::stop::{
    assistant_content_from_api_message, extract_reasoning_content, extract_tool_calls,
    is_tools_unsupported_error, sanitize_assistant_text_response,
};
use super::stream::{ToolCallDraftTracker, WebSearchCardTracker};
use super::types::{AgentRunResult, AgentStreamPolicy};

pub(crate) struct ChatPlanningStep {
    pub(crate) message: Value,
    pub(crate) streamed: bool,
}

pub(crate) struct PlannedToolRound {
    pub(crate) message: Value,
    pub(crate) tool_calls: Vec<PendingToolCall>,
}

pub(crate) enum PlanningStepOutcome {
    /// `state.planning_final_message` / `planning_final_streamed` were written;
    /// the skeleton breaks out of the tool loop.
    FinalAnswer,
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
        super::compaction::maybe_compact_send_view(env, state, &active_tools).await;

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
            ChatMessageSegmentKind::Text,
            ChatMessageSegmentPhase::Plain,
            Some(step_number),
            Some(round),
            &format!("step_{step_number}_compaction_blocked"),
        );
        return Ok(PlanningStepOutcome::Recovered(
            RunResultBuilder::new(host, env.ids(), content)
                .segment(&degrade_segment)
                .emit_done("done")
                .outcome("compaction_blocked")
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
        ChatMessageSegmentKind::Reasoning,
        ChatMessageSegmentPhase::ToolLoop,
        Some(step_number),
        Some(round),
        &format!("step_{step_number}_reasoning"),
    );
    // 在 reasoning 与正文之间预留内置搜索实时卡的 order 槽（任务 07-23）：搜索发生时卡插到
    // 这里 → 渲染在答案文本之前；未搜索则该 order 留空洞无害。
    let web_search_order = state.segment_builder.reserve_order();
    let planning_text_segment = state.segment_builder.reserve(
        ChatMessageSegmentKind::Text,
        ChatMessageSegmentPhase::ToolLoop,
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
    // 内置搜索实时卡追踪器：仅在会话 Builtin 模式且模型支持时建（否则 None，走现状路径）。
    let planning_web_search_tracker = if config.builtin_web_search_active() {
        Some(WebSearchCardTracker::new(
            web_search_order,
            Some(round),
            ChatMessageSegmentPhase::ToolLoop,
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
                retry_attempts: config.retry_attempts,
                thinking_enabled: config.thinking_enabled,
                thinking_level: config.thinking_level.clone(),
                builtin_web_search: config.builtin_web_search_active(),
                max_output_tokens: config.max_output_tokens,
                conversation_id: &config.conversation_id,
                run_id: &config.run_id,
                message_id: &config.message_id,
                generation: config.generation,
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
                    let partial = sanitize_assistant_text_response(&stream.content);
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
                        reasoning_segment.phase = ChatMessageSegmentPhase::Plain;
                        state.segment_builder.append_text_from_template(
                            &reasoning_segment,
                            reasoning_text.to_string(),
                        );
                    }
                    let mut segment = planning_text_segment.clone();
                    segment.phase = ChatMessageSegmentPhase::Plain;
                    return Ok(PlanningStepOutcome::Cancelled(
                        RunResultBuilder::new(host, env.ids(), partial)
                            .segment(&segment)
                            .api_reasoning(stream.reasoning.clone())
                            .reasoning_tail(stream.reasoning)
                            .outcome("cancelled")
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
                break Ok(ChatPlanningStep {
                    message: stream.to_openai_compatible_message(),
                    streamed: true,
                });
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
            Err(err)
                if !overflow_attempted
                    && super::recovery::classify(&err.to_string())
                        == super::recovery::FailureKind::ContextOverflow =>
            {
                overflow_attempted = true;
                let previous = send_messages.clone();
                send_messages =
                    super::compaction::compact_send_view(env, state, true, &active_tools).await;
                if state.compaction_blocked || previous == send_messages {
                    break Err(err.to_string());
                }
                continue;
            }
            Err(err) => break Err(err.to_string()),
        }
    };
    let message = match planning_result {
        Ok(step) => {
            state.planning_final_streamed = step.streamed;
            step.message
        }
        Err(err) if is_tools_unsupported_error(&err) => {
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
                let content = super::synthesis::recover_synthesis(env, state, &err).await;
                if !content.trim().is_empty() {
                    eprintln!("Chat planning call failed mid-run; recovered: {err}");
                    // 降级文案是本轮的**正文**，段 phase 必须是 Synthesis（不能留 ToolLoop）：
                    // content_from_segments 只认 Plain|Synthesis，留 ToolLoop 会让
                    // normalize_assistant_segments 以为正文没落段而再补一条，正文渲染两遍。
                    let mut segment = planning_text_segment.clone();
                    segment.phase = ChatMessageSegmentPhase::Synthesis;
                    return Ok(PlanningStepOutcome::Recovered(
                        RunResultBuilder::new(host, env.ids(), content)
                            .segment(&segment)
                            .emit_done("done")
                            .outcome("recovered")
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
            return Err(err);
        }
    };
    // 内置搜索：实时卡追踪器边流边合成（take_card 落 Success 终态卡）。
    if let Some(tracker) = planning_web_search_tracker.as_ref() {
        if let Some((record, segment)) = tracker.take_card() {
            state
                .segment_builder
                .append_existing_segments(vec![segment]);
            state.tool_records.push(record);
        }
    }
    let tool_calls = extract_tool_calls(&message);
    if tool_calls.is_empty() {
        let response =
            sanitize_assistant_text_response(&assistant_content_from_api_message(&message));
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
            segment.phase = ChatMessageSegmentPhase::Plain;
            if state.planning_final_streamed {
                host.emit_stream_delta(
                    &config.conversation_id,
                    &config.run_id,
                    &config.message_id,
                    "",
                    None,
                    Some(&segment),
                );
            }
            state
                .segment_builder
                .append_text_from_template(&segment, response);
        }
        if let Some(reasoning) = extract_reasoning_content(&message) {
            let mut segment = planning_reasoning_segment.clone();
            segment.phase = ChatMessageSegmentPhase::Plain;
            if state.planning_final_streamed {
                host.emit_stream_delta(
                    &config.conversation_id,
                    &config.run_id,
                    &config.message_id,
                    "",
                    None,
                    Some(&segment),
                );
            }
            state
                .segment_builder
                .append_text_from_template(&segment, reasoning);
        }
        state.planning_final_message = Some(message);
        return Ok(PlanningStepOutcome::FinalAnswer);
    }
    state.planning_final_streamed = false;
    let planning_text =
        sanitize_assistant_text_response(&assistant_content_from_api_message(&message));
    if !planning_text.trim().is_empty() {
        state
            .segment_builder
            .append_text_from_template(&planning_text_segment, planning_text);
    }
    if let Some(reasoning) = extract_reasoning_content(&message) {
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
            ChatMessageSegmentPhase::ToolLoop,
            Some(step_number),
            round,
            &visible_tool_calls,
        );
        for segment in &tool_segments {
            host.emit_stream_delta(
                &config.conversation_id,
                &config.run_id,
                &config.message_id,
                "",
                None,
                Some(segment),
            );
        }
    } else {
        state
            .segment_builder
            .append_existing_segments(draft_tool_segments);
    }
    Ok(PlanningStepOutcome::ToolCalls(PlannedToolRound {
        message,
        tool_calls,
    }))
}

pub(crate) use super::model_calls::{
    call_chat_completion_message_streamed, call_chat_completion_message_with_usage,
    stream_scoped_chat_completion_inner,
};
