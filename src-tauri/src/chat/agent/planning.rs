use serde_json::Value;

use crate::chat::model::{
    generate_request_from_openai_messages, AnthropicMessagesProvider, GenerateOptions,
    GenerateOutput, GenerateRequestContext, LanguageModelProvider, ModelError, OpenAiChatProvider,
    PendingToolCall,
};
use crate::chat::types::{ChatMessageSegment, ChatMessageSegmentKind, ChatMessageSegmentPhase};
use crate::mcp::ChatToolDefinition;
use crate::settings::ProviderApiFormat;

use super::finalize::{tool_planning_failed_run_result, RunResultBuilder};
use super::host::AgentHost;
use super::loop_::{LoopEnv, RunState};
use super::prepare::{prepare_agent_step, PrepareStepInput};
use super::rounds::visible_tool_segment_calls;
use super::stop::{
    assistant_content_from_api_message, extract_reasoning_content, extract_tool_calls,
    is_tools_unsupported_error, sanitize_assistant_text_response,
};
use super::stream::{
    should_emit_done, validate_stream_output, AgentStreamSink, ChatStreamOutput,
    ToolCallDraftTracker,
};
use super::types::{
    AgentPhase, AgentRunResult, AgentStepResult, AgentStopReason, AgentStreamPolicy,
};

pub(crate) struct ChatPlanningStep {
    pub(crate) message: Value,
    pub(crate) streamed: bool,
}

pub(crate) struct PlannedToolRound {
    pub(crate) message: Value,
    pub(crate) tool_calls: Vec<PendingToolCall>,
    pub(crate) step_segments: Vec<ChatMessageSegment>,
}

pub(crate) enum PlanningStepOutcome {
    /// `state.planning_final_message` / `planning_final_streamed` were written;
    /// the skeleton breaks out of the tool loop.
    FinalAnswer,
    /// Planning produced tool calls; the skeleton hands them to `run_tool_round`.
    ToolCalls(PlannedToolRound),
    /// `state.tools` was narrowed to skill-native tools; the skeleton retries.
    RetryWithSkillTools,
    /// Provider rejected tools; `state.provider_tools_unsupported` was set and a
    /// step was pushed. The skeleton breaks out of the tool loop.
    ToolsUnsupported,
    /// Tool-call argument drafting failed mid-stream; the run ends immediately.
    DraftFailed(AgentRunResult),
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
    let prepared = prepare_agent_step(PrepareStepInput {
        step_number,
        previous_steps: &state.steps,
        runtime_messages: &state.runtime_messages,
        tools: &state.tools,
        phase: AgentPhase::ToolLoop,
    });
    let planning_reasoning_segment = state.segment_builder.reserve(
        ChatMessageSegmentKind::Reasoning,
        ChatMessageSegmentPhase::ToolLoop,
        Some(step_number),
        Some(round),
        &format!("step_{step_number}_reasoning"),
    );
    let planning_text_segment = state.segment_builder.reserve(
        ChatMessageSegmentKind::Text,
        ChatMessageSegmentPhase::ToolLoop,
        Some(step_number),
        Some(round),
        &format!("step_{step_number}_text"),
    );
    let planning_tool_drafts = ToolCallDraftTracker::new(
        prepared.active_tools.clone(),
        round,
        Some(step_number),
        state.segment_builder.next_order(),
    );
    let planning_result = if config.stream_enabled {
        match stream_scoped_chat_completion_inner(
            config.state,
            host,
            &config.provider,
            &config.model,
            prepared.runtime_messages.clone(),
            Some(&prepared.active_tools),
            config.retry_attempts,
            config.thinking_enabled,
            config.max_output_tokens,
            &config.conversation_id,
            &config.run_id,
            &config.message_id,
            config.generation,
            "Chat tools planning",
            prepared.stream_policy,
            Some(planning_text_segment.clone()),
            Some(planning_reasoning_segment.clone()),
            Some(planning_tool_drafts.clone()),
        )
        .await
        {
            Ok(stream) => {
                if stream.cancelled {
                    let partial = sanitize_assistant_text_response(&stream.content);
                    if partial.trim().is_empty() || planning_tool_drafts.has_started() {
                        return Err("cancelled".to_string());
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
                        state
                            .segment_builder
                            .append_text_from_template(&reasoning_segment, reasoning_text.to_string());
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
                                std::mem::take(&mut state.steps),
                            ),
                    ));
                }
                Ok(ChatPlanningStep {
                    message: stream.to_openai_compatible_message(),
                    streamed: true,
                })
            }
            Err(err) if planning_tool_drafts.has_started() => {
                eprintln!(
                    "Chat tools planning stream interrupted while generating tool arguments; surfacing tool draft error without retry: {}",
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
                        std::mem::take(&mut state.steps),
                        err.to_string(),
                    ),
                ));
            }
            Err(err) if err.is_stream_read_interrupted() => {
                eprintln!(
                    "Chat tools planning stream interrupted; retrying once without streaming: {}",
                    err
                );
                call_chat_completion_message(
                    config.state,
                    &config.provider,
                    &config.model,
                    prepared.runtime_messages.clone(),
                    Some(&prepared.active_tools),
                    config.retry_attempts,
                    config.thinking_enabled,
                    config.max_output_tokens,
                    &config.conversation_id,
                    &config.message_id,
                    "Chat tools planning",
                )
                .await
                .map(|message| ChatPlanningStep {
                    message,
                    streamed: false,
                })
            }
            Err(err) => Err(err.to_string()),
        }
    } else {
        tokio::select! {
            result = call_chat_completion_message(
                config.state,
                &config.provider,
                &config.model,
                prepared.runtime_messages.clone(),
                Some(&prepared.active_tools),
                config.retry_attempts,
                config.thinking_enabled,
                config.max_output_tokens,
                &config.conversation_id,
                &config.message_id,
                "Chat tools planning",
            ) => result.map(|message| ChatPlanningStep {
                message,
                streamed: false,
            }),
            _ = host.wait_for_generation_inactive(&config.conversation_id, config.generation) => {
                host.emit_stream_done(
                    &config.conversation_id,
                    &config.run_id,
                    &config.message_id,
                    "cancelled",
                    "",
                );
                return Err("cancelled".to_string());
            }
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
            state.steps.push(AgentStepResult {
                step_number,
                phase: AgentPhase::ToolLoop,
                response_messages: Vec::new(),
                tool_records: Vec::new(),
                segments: Vec::new(),
                streamed: false,
                stop_reason: Some(AgentStopReason::ProviderToolsUnsupported),
            });
            return Ok(PlanningStepOutcome::ToolsUnsupported);
        }
        Err(err) => return Err(err),
    };
    let tool_calls = extract_tool_calls(&message);
    if tool_calls.is_empty() {
        let response =
            sanitize_assistant_text_response(&assistant_content_from_api_message(&message));
        let mut step_segments = Vec::new();
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
            if let Some(segment) = state
                .segment_builder
                .append_text_from_template(&segment, response)
            {
                step_segments.push(segment);
            }
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
            if let Some(segment) = state
                .segment_builder
                .append_text_from_template(&segment, reasoning)
            {
                step_segments.push(segment);
            }
        }
        state.planning_final_message = Some(message.clone());
        state.steps.push(AgentStepResult {
            step_number,
            phase: AgentPhase::ToolLoop,
            response_messages: vec![message],
            tool_records: Vec::new(),
            segments: step_segments,
            streamed: state.planning_final_streamed,
            stop_reason: Some(AgentStopReason::Natural),
        });
        return Ok(PlanningStepOutcome::FinalAnswer);
    }
    state.planning_final_streamed = false;
    let planning_text =
        sanitize_assistant_text_response(&assistant_content_from_api_message(&message));
    let mut step_segments = Vec::new();
    if !planning_text.trim().is_empty() {
        if let Some(segment) = state
            .segment_builder
            .append_text_from_template(&planning_text_segment, planning_text)
        {
            step_segments.push(segment);
        }
    }
    if let Some(reasoning) = extract_reasoning_content(&message) {
        if !config.stream_enabled {
            host.emit_stream_delta(
                &config.conversation_id,
                &config.run_id,
                &config.message_id,
                "",
                Some(&reasoning),
                Some(&planning_reasoning_segment),
            );
        }
        if let Some(segment) = state
            .segment_builder
            .append_text_from_template(&planning_reasoning_segment, reasoning.clone())
        {
            step_segments.push(segment);
        }
        state.planning_reasoning_parts.push(reasoning);
    }

    let visible_tool_calls =
        visible_tool_segment_calls(&state.tools, &state.blocked_tool_calls, &tool_calls);
    let draft_tool_segments = planning_tool_drafts.segments();
    let tool_segments = if draft_tool_segments.is_empty() {
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
        tool_segments
    } else {
        state
            .segment_builder
            .append_existing_segments(draft_tool_segments)
    };
    step_segments.extend(tool_segments);
    Ok(PlanningStepOutcome::ToolCalls(PlannedToolRound {
        message,
        tool_calls,
        step_segments,
    }))
}

pub(crate) async fn call_chat_completion_message(
    state: &crate::state::AppState,
    provider: &crate::settings::ModelProvider,
    model: &str,
    messages: Vec<Value>,
    tools: Option<&[ChatToolDefinition]>,
    retry_attempts: usize,
    thinking_enabled: bool,
    max_output_tokens: u32,
    conversation_id: &str,
    message_id: &str,
    label: &str,
) -> Result<Value, String> {
    let request = generate_request_from_openai_messages(
        model,
        messages,
        tools,
        GenerateOptions {
            thinking_enabled,
            max_tokens: max_output_tokens,
            ..GenerateOptions::default()
        },
        label,
        GenerateRequestContext::new(Some(conversation_id), Some(message_id)),
    );
    let output = generate_with_chat_provider(state, provider, retry_attempts, request)
        .await
        .map_err(|err| err.to_string())?;
    Ok(output.to_openai_compatible_message())
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn stream_scoped_chat_completion_inner(
    state: &crate::state::AppState,
    host: &dyn AgentHost,
    provider: &crate::settings::ModelProvider,
    model: &str,
    messages: Vec<Value>,
    tools: Option<&[ChatToolDefinition]>,
    retry_attempts: usize,
    thinking_enabled: bool,
    max_output_tokens: u32,
    conversation_id: &str,
    run_id: &str,
    message_id: &str,
    generation: u64,
    label: &str,
    policy: AgentStreamPolicy,
    text_segment: Option<ChatMessageSegment>,
    reasoning_segment: Option<ChatMessageSegment>,
    tool_draft_tracker: Option<ToolCallDraftTracker>,
) -> Result<ChatStreamOutput, ModelError> {
    let request = generate_request_from_openai_messages(
        model,
        messages,
        tools,
        GenerateOptions {
            stream: true,
            thinking_enabled,
            max_tokens: max_output_tokens,
            ..GenerateOptions::default()
        },
        label,
        GenerateRequestContext::new(Some(conversation_id), Some(message_id)),
    );
    let mut sink = AgentStreamSink::new(
        host,
        conversation_id,
        run_id,
        message_id,
        matches!(policy, AgentStreamPolicy::PlanningNoDoneUntilNoTools),
        text_segment,
        reasoning_segment,
        tool_draft_tracker.clone(),
    );
    let output = tokio::select! {
        result = stream_with_chat_provider(
            state,
            provider,
            retry_attempts,
            request,
            &mut sink,
        ) => result?,
        _ = host.wait_for_generation_inactive(conversation_id, generation) => {
            let (content, reasoning) = sink.snapshot();
            host.emit_stream_done(
                conversation_id,
                run_id,
                message_id,
                "cancelled",
                content.trim(),
            );
            return Ok(ChatStreamOutput::new(
                content.trim().to_string(),
                reasoning.trim().to_string(),
                true,
            ));
        }
    };
    sink.flush_pending_text();
    let (snapshot_content, snapshot_reasoning) = sink.snapshot();
    let stream_output = ChatStreamOutput::from_generate_output_with_snapshot(
        output,
        snapshot_content,
        snapshot_reasoning,
    );
    validate_stream_output(label, policy, &stream_output).map_err(|err| {
        if !tool_draft_tracker
            .as_ref()
            .is_some_and(|tracker| tracker.has_unfinished_drafts())
        {
            host.emit_stream_done(conversation_id, run_id, message_id, "error", "");
        }
        ModelError::new(err)
    })?;
    if should_emit_done(policy, &stream_output) {
        sink.flush_pending_text();
        host.emit_stream_done(
            conversation_id,
            run_id,
            message_id,
            "done",
            &stream_output.content,
        );
    }
    Ok(stream_output)
}
pub(crate) async fn generate_with_chat_provider(
    state: &crate::state::AppState,
    provider: &crate::settings::ModelProvider,
    retry_attempts: usize,
    request: crate::chat::model::GenerateRequest,
) -> Result<GenerateOutput, ModelError> {
    match provider.api_format_kind() {
        ProviderApiFormat::OpenAiChat => {
            OpenAiChatProvider::new(state, provider, retry_attempts)
                .generate(request)
                .await
        }
        ProviderApiFormat::AnthropicMessages => {
            AnthropicMessagesProvider::new(state, provider, retry_attempts)
                .generate(request)
                .await
        }
    }
}

pub(crate) async fn stream_with_chat_provider(
    state: &crate::state::AppState,
    provider: &crate::settings::ModelProvider,
    retry_attempts: usize,
    request: crate::chat::model::GenerateRequest,
    sink: &mut (dyn crate::chat::model::StreamSink + Send),
) -> Result<GenerateOutput, ModelError> {
    match provider.api_format_kind() {
        ProviderApiFormat::OpenAiChat => {
            OpenAiChatProvider::new(state, provider, retry_attempts)
                .stream(request, sink)
                .await
        }
        ProviderApiFormat::AnthropicMessages => {
            AnthropicMessagesProvider::new(state, provider, retry_attempts)
                .stream(request, sink)
                .await
        }
    }
}
