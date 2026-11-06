use super::content::ContentKind;
use super::history::RuntimeMessage;
use crate::model::{
    GenerateOutput, ModelError, ModelErrorKind, ModelMessage, StreamPart, StreamSink,
};

use super::finalize::{segment_phase_for_agent_phase, RunResultBuilder};
use super::loop_::{LoopEnv, RunState};
use super::provider_runtime::{ModelCall, ModelDelivery, ModelOutputSink};
use super::recovery::{self, FallbackReason, RecoveryDecision, RecoveryFailure, RecoveryInput};
use super::session::SessionPolicy;
use super::stop::{
    empty_assistant_response_error, merge_reasoning, sanitize_assistant_text_response,
};
use super::stream::WebSearchCardTracker;
use super::types::{AgentPhase, AgentRunConfig, AgentRunResult, AgentStreamPolicy};

pub(crate) struct SynthesisCompleted {
    pub(crate) replay: Option<crate::model::ModelMessage>,
    pub(crate) response: String,
    pub(crate) reasoning: Option<String>,
    pub(crate) response_reasoning: Option<String>,
    pub(crate) generated_message: Option<RuntimeMessage>,
}

pub(crate) enum SynthesisFlow {
    Completed(SynthesisCompleted),
    Early(AgentRunResult),
    CancelledState,
}

pub(crate) async fn synthesis_step(
    env: &LoopEnv<'_>,
    state: &mut RunState,
    session: &mut dyn SessionPolicy,
) -> Result<SynthesisFlow, String> {
    let config = env.config;
    let host = env.host;
    state.step_number = state.step_number.saturating_add(1);
    let step_number = state.step_number;
    let phase = if state.tool_records.is_empty() && !state.provider_tools_unsupported {
        AgentPhase::Plain
    } else {
        AgentPhase::Synthesis
    };
    // 循环内上下文治理：超限时先 snip / 摘要（与 planning_step 相同的发送视图）。
    let send_messages =
        super::compaction::maybe_compact_send_view(env, state, &[], session).await?;
    if state.compaction_blocked {
        let content = session.context_blocked(recovery::snapshot(state));
        let segment = state.segment_builder.reserve(
            ContentKind::Text,
            segment_phase_for_agent_phase(phase),
            Some(step_number),
            None,
            &format!("step_{step_number}_compaction_blocked"),
        );
        return Ok(SynthesisFlow::Early(
            RunResultBuilder::new(host, content)
                .segment(&segment)
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
    let synthesis_stream_policy = if state.tool_records.is_empty() {
        AgentStreamPolicy::SynthesisAlwaysDone
    } else {
        AgentStreamPolicy::SynthesisDeferEmpty
    };
    let response_phase = segment_phase_for_agent_phase(phase);
    let response_reasoning_segment = state.segment_builder.reserve(
        ContentKind::Reasoning,
        response_phase.clone(),
        Some(step_number),
        None,
        &format!("step_{step_number}_reasoning"),
    );
    // reasoning 与正文之间预留内置搜索实时卡的 order 槽（任务 07-23，与 planning 同款）。
    let web_search_order = state.segment_builder.reserve_order();
    let response_segment = state.segment_builder.reserve(
        ContentKind::Text,
        response_phase.clone(),
        Some(step_number),
        None,
        &format!("step_{step_number}_text"),
    );
    // 内置搜索实时卡追踪器：仅 Builtin 模式且模型支持时建（否则 None，走现状路径）。
    let synth_web_search_tracker = if config.builtin_web_search_active() {
        Some(WebSearchCardTracker::new(
            web_search_order,
            None,
            response_phase.clone(),
            config.provider_label.clone(),
        ))
    } else {
        None
    };

    let stream = recovery::stream_with_truncation_recovery(
        env,
        state,
        recovery::StreamRequest {
            call: config.model_call(
                send_messages,
                Vec::new(),
                crate::model::ToolChoice::Auto,
                "Chat stream",
                ModelDelivery::Streamed,
            ),
            policy: synthesis_stream_policy,
            text_segment: Some(response_segment.clone()),
            reasoning_segment: Some(response_reasoning_segment.clone()),
            tool_draft_tracker: None,
            web_search_tracker: synth_web_search_tracker.clone(),
            speculation: None,
        },
        session,
    )
    .await;
    let mut stream = match stream {
        Ok(stream) => stream,
        Err(err) if !state.tool_records.is_empty() => {
            eprintln!("Chat synthesis stream failed after tool records; recovering: {err}");
            let recovered = recover_synthesis(env, state, &err, session).await?;
            let replay = recovered.replay;
            let content = if recovered.text.trim().is_empty() {
                session.fallback(FallbackReason::FailedSynthesis)
            } else {
                recovered.text
            };
            return Ok(SynthesisFlow::Early(
                RunResultBuilder::new(host, content)
                    .segment(&response_segment)
                    .emit_done("done")
                    .outcome(super::types::AgentRunStatus::Recovered)
                    .replay(replay.clone())
                    .finish(
                        std::mem::take(&mut state.segment_builder),
                        &state.planning_reasoning_parts,
                        std::mem::take(&mut state.tool_records),
                        std::mem::take(&mut state.generated_api_messages),
                    ),
            ));
        }
        Err(err) => return Err(err.message),
    };
    if stream.cancelled {
        let partial = if state.tool_records.is_empty() {
            sanitize_assistant_text_response(&stream.text)
        } else {
            stream.text.clone()
        };
        if partial.trim().is_empty()
            && stream
                .reasoning
                .as_deref()
                .is_none_or(|reasoning| reasoning.trim().is_empty())
            && stream.canonical_message.is_none()
            && stream.reasoning_items.is_empty()
        {
            return Ok(SynthesisFlow::CancelledState);
        }
        if let Some(reasoning) = stream.reasoning.as_deref() {
            state
                .segment_builder
                .append_text_from_template(&response_reasoning_segment, reasoning.trim());
        }
        let replay = stream.replay_message();
        return Ok(SynthesisFlow::Early(
            RunResultBuilder::new(host, partial)
                .segment(&response_segment)
                .api_reasoning(stream.reasoning.clone())
                .reasoning_tail(stream.reasoning)
                .replay(Some(replay))
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
    let final_reasoning_for_api = stream.reasoning.clone();
    let reasoning = merge_reasoning(&state.planning_reasoning_parts, stream.reasoning.clone());
    let response = sanitize_assistant_text_response(&stream.text);
    let replay;
    let generated_message;
    let (response, reasoning, response_reasoning) = if response.trim().is_empty() {
        if !state.tool_records.is_empty() {
            log_empty_synthesis_output(config, phase, &stream, state.tool_records.len());
            let recovered = recover_synthesis(
                env,
                state,
                &ModelError::with_kind("", ModelErrorKind::EmptyResponse),
                session,
            )
            .await?;
            replay = recovered.replay;
            let content = if recovered.text.trim().is_empty() {
                session.fallback(FallbackReason::EmptySynthesis)
            } else {
                recovered.text
            };
            host.content(&content, None, Some(&response_segment));
            generated_message = (!state.generated_api_messages.is_empty()).then(|| {
                RuntimeMessage::model(replay.clone().unwrap_or_else(|| {
                    super::stop::final_assistant_message(
                        &content,
                        final_reasoning_for_api.as_deref(),
                    )
                }))
            });
            (content, reasoning, final_reasoning_for_api)
        } else {
            return Err(empty_assistant_response_error("Chat stream"));
        }
    } else {
        let message = stream.replay_message();
        generated_message = (!state.generated_api_messages.is_empty()
            || stream.canonical_message.is_some()
            || !stream.reasoning_items.is_empty())
        .then(|| RuntimeMessage::model(message.clone()));
        replay = Some(message);
        (response, reasoning, final_reasoning_for_api)
    };

    // 内置搜索：实时卡追踪器边流边合成（take_card 落 Success 终态卡）。
    if let Some(tracker) = synth_web_search_tracker.as_ref() {
        if let Some(segment) = tracker.take_card() {
            state
                .segment_builder
                .append_existing_segments(vec![segment]);
        }
    }

    state
        .segment_builder
        .append_text_from_template(&response_segment, response.clone());
    if let Some(reasoning_part) = response_reasoning.as_deref() {
        state
            .segment_builder
            .append_text_from_template(&response_reasoning_segment, reasoning_part.trim());
    }

    Ok(SynthesisFlow::Completed(SynthesisCompleted {
        replay,
        response,
        reasoning,
        response_reasoning,
        generated_message,
    }))
}

pub(crate) struct RecoveredAnswer {
    pub text: String,
    pub replay: Option<ModelMessage>,
}

impl RecoveredAnswer {
    fn fallback(text: String) -> Self {
        Self { text, replay: None }
    }

    fn from_model(output: GenerateOutput) -> Self {
        let replay = Some(output.replay_message());
        Self {
            text: output.text,
            replay,
        }
    }
}

pub(crate) async fn recover_synthesis(
    env: &LoopEnv<'_>,
    state: &mut RunState,
    failure: &ModelError,
    session: &mut dyn SessionPolicy,
) -> Result<RecoveredAnswer, String> {
    let input = match session.recover(recovery::snapshot(state), failure) {
        RecoveryDecision::Surface => return Err(failure.message.clone()),
        RecoveryDecision::Complete(text) => return Ok(RecoveredAnswer::fallback(text)),
        RecoveryDecision::Retry(input) => input,
    };
    let (history, display) = match input {
        RecoveryInput::CompactCurrent => {
            let history =
                super::compaction::compact_send_view(env, state, true, &[], session).await?;
            if state.compaction_blocked {
                return Ok(RecoveredAnswer::fallback(session.recovery_failed(
                    recovery::snapshot(state),
                    RecoveryFailure::Blocked,
                )));
            }
            (history, true)
        }
        RecoveryInput::Reduced(history) => (history, false),
    };
    let label = if display {
        "Chat synthesis overflow recovery"
    } else {
        "Chat synthesis recovery"
    };
    let call = env.config.model_call(
        history,
        Vec::new(),
        crate::model::ToolChoice::Auto,
        label,
        ModelDelivery::Collected,
    );
    let mut sink = CollectedRecoverySink {
        host: env.host,
        report_context: display,
    };
    let result = tokio::select! {
        result = env.config.provider_runtime.generate(call, &mut sink) => result,
        _ = env.host.control.wait_inactive() => Err(ModelError::with_kind("cancelled", ModelErrorKind::Cancelled)),
    };
    let failure = match result {
        Ok(mut output) => {
            state.merge_usage_with_billing(
                output.usage.clone(),
                output.billing_usage.clone(),
                env,
                display,
            );
            output.text = sanitize_assistant_text_response(&output.text);
            if !output.text.trim().is_empty() {
                state.generated_images.append(&mut output.images);
                return Ok(RecoveredAnswer::from_model(output));
            }
            None
        }
        Err(error) => Some(error),
    };
    Ok(RecoveredAnswer::fallback(
        session.recovery_failed(
            recovery::snapshot(state),
            failure
                .as_ref()
                .map(RecoveryFailure::Error)
                .unwrap_or(RecoveryFailure::Empty),
        ),
    ))
}

struct CollectedRecoverySink<'a> {
    host: &'a super::context::AgentContext<'a>,
    report_context: bool,
}

impl StreamSink for CollectedRecoverySink<'_> {
    fn emit(&mut self, _: StreamPart) -> Result<(), ModelError> {
        Ok(())
    }
}

impl ModelOutputSink for CollectedRecoverySink<'_> {
    fn request_started(&mut self, call: &ModelCall) {
        if self.report_context {
            self.host
                .context_prepared(&call.provider_id, &call.model, &call.history, &call.tools);
        }
    }
    fn retry(&mut self, _: super::events::RetryStatus) {}
    fn reset_attempt(&mut self) {}
    fn draft_snapshot(&self) -> Vec<(String, String)> {
        Vec::new()
    }
}

fn log_empty_synthesis_output(
    config: &AgentRunConfig<'_>,
    phase: AgentPhase,
    stream: &GenerateOutput,
    tool_record_count: usize,
) {
    eprintln!(
        "Chat agent empty synthesis fallback: conversation_id={} run_id={} provider_id={} model={} phase={:?} stream=true tool_records={} finish_reason={} text_chars={} reasoning_chars={} stream_tool_calls={}",
        config.conversation_id,
        config.run_id,
        config.provider_id,
        config.model,
        phase,
        tool_record_count,
        stream.finish_reason.as_ref().map(|reason| reason.raw.as_str()).unwrap_or("unknown"),
        stream.text.chars().count(),
        stream.reasoning.as_deref().map(|value| value.chars().count()).unwrap_or(0),
        stream.tool_calls.len(),
    );
}
