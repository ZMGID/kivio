use serde_json::Value;

use crate::chat::model::{
    generate_request_from_openai_messages, AnthropicMessagesProvider, GenerateOptions,
    GenerateOutput, GenerateRequestContext, LanguageModelProvider, ModelError, OpenAiChatProvider,
    PendingToolCall,
};
use crate::chat::types::{
    ChatMessageSegment, ChatMessageSegmentKind, ChatMessageSegmentPhase, ToolCallRecord,
    ToolCallStatus,
};
use crate::mcp::ChatToolDefinition;
use crate::settings::{ProviderApiFormat, Settings};
use crate::skills;

use super::execute::{
    disabled_tool_content, execute_tool_call, invalid_tool_arguments_record, match_tool_call,
    tool_requires_approval, unknown_tool_record, ToolExecutionContext, ToolExecutor,
};
use super::host::AgentHost;
use super::prepare::{prepare_agent_step, PrepareStepInput};
use super::stop::{
    assistant_api_message_for_tool_calls, assistant_content_from_api_message,
    empty_assistant_response_error, extract_reasoning_content, extract_tool_calls,
    final_assistant_api_message, final_response_from_planning_message, is_tools_unsupported_error,
    merge_reasoning, patch_system_message, sanitize_assistant_text_response,
    step_limit_system_message,
};
use super::stream::{
    should_emit_done, validate_stream_output, AgentStreamSink, ChatStreamOutput,
    ToolCallDraftTracker,
};
use super::types::{
    AgentPhase, AgentRunConfig, AgentRunResult, AgentStepResult, AgentStopReason, AgentStreamPolicy,
};

struct ChatPlanningStep {
    message: Value,
    streamed: bool,
}

const MAX_PARALLEL_TOOL_CALLS_PER_ROUND: usize = 4;

struct ToolRoundContext<'a> {
    conversation_id: &'a str,
    run_id: &'a str,
    message_id: &'a str,
    generation: u64,
    round: u32,
}

struct ToolRoundResult {
    response_messages: Vec<Value>,
    tool_records: Vec<ToolCallRecord>,
    cancelled: bool,
}

struct ToolExecutionResult {
    response_message: Value,
    record: Option<ToolCallRecord>,
    cancelled: bool,
}

struct ExecutableToolCall<'a> {
    call: PendingToolCall,
    tool: &'a ChatToolDefinition,
}

struct SegmentBuilder {
    next_order: u32,
    segments: Vec<ChatMessageSegment>,
}

impl SegmentBuilder {
    fn new() -> Self {
        Self {
            next_order: 1000,
            segments: Vec::new(),
        }
    }

    fn reserve(
        &mut self,
        kind: ChatMessageSegmentKind,
        phase: ChatMessageSegmentPhase,
        step_number: Option<u8>,
        round: Option<u32>,
        suffix: &str,
    ) -> ChatMessageSegment {
        let segment = ChatMessageSegment {
            id: format!("seg_{}_{}", self.next_order, suffix),
            kind,
            phase,
            order: self.next_order,
            step_number,
            round,
            text: None,
            tool_call_id: None,
        };
        self.next_order = self.next_order.saturating_add(1);
        segment
    }

    fn append_text_from_template(
        &mut self,
        template: &ChatMessageSegment,
        text: impl Into<String>,
    ) -> Option<ChatMessageSegment> {
        let text = text.into();
        if text.trim().is_empty() {
            return None;
        }
        let mut segment = template.clone();
        segment.text = Some(text);
        self.segments.push(segment.clone());
        Some(segment)
    }

    fn append_tool_calls(
        &mut self,
        phase: ChatMessageSegmentPhase,
        step_number: Option<u8>,
        round: u32,
        calls: &[PendingToolCall],
    ) -> Vec<ChatMessageSegment> {
        let mut segments = Vec::new();
        for call in calls {
            let segment = ChatMessageSegment {
                id: format!("seg_{}_tool_{}", self.next_order, call.id),
                kind: ChatMessageSegmentKind::Tool,
                phase: phase.clone(),
                order: self.next_order,
                step_number,
                round: Some(round),
                text: None,
                tool_call_id: Some(call.id.clone()),
            };
            self.next_order = self.next_order.saturating_add(1);
            self.segments.push(segment.clone());
            segments.push(segment);
        }
        segments
    }

    fn append_existing_segments(
        &mut self,
        mut segments: Vec<ChatMessageSegment>,
    ) -> Vec<ChatMessageSegment> {
        for segment in &segments {
            self.next_order = self.next_order.max(segment.order.saturating_add(1));
        }
        self.segments.extend(segments.iter().cloned());
        segments.sort_by_key(|segment| segment.order);
        segments
    }

    fn next_order(&self) -> u32 {
        self.next_order
    }

    fn all(self) -> Vec<ChatMessageSegment> {
        self.segments
    }
}

fn segment_phase_for_agent_phase(phase: AgentPhase) -> ChatMessageSegmentPhase {
    match phase {
        AgentPhase::Plain => ChatMessageSegmentPhase::Plain,
        AgentPhase::Synthesis => ChatMessageSegmentPhase::Synthesis,
        AgentPhase::ToolLoop => ChatMessageSegmentPhase::ToolLoop,
    }
}

fn visible_tool_segment_calls(
    tools: &[ChatToolDefinition],
    blocked_tool_calls: &[ChatToolDefinition],
    tool_calls: &[PendingToolCall],
) -> Vec<PendingToolCall> {
    tool_calls
        .iter()
        .filter(|call| {
            match_tool_call(tools, &call.function_name).is_some()
                || match_tool_call(blocked_tool_calls, &call.function_name).is_some()
                || disabled_tool_content(call).is_none()
        })
        .cloned()
        .collect()
}

pub async fn run_agent_loop(
    mut config: AgentRunConfig<'_>,
    host: &dyn AgentHost,
    executor: &dyn ToolExecutor,
) -> Result<AgentRunResult, String> {
    let mut runtime_messages = std::mem::take(&mut config.runtime_messages);
    let mut tools = std::mem::take(&mut config.tools);
    let blocked_tool_calls = std::mem::take(&mut config.blocked_tool_calls);
    let mut generated_api_messages = Vec::new();
    let mut tool_records = Vec::new();
    let mut planning_reasoning_parts: Vec<String> = Vec::new();
    let mut provider_tools_unsupported = false;
    let mut planning_final_message: Option<Value> = None;
    let mut planning_final_already_streamed = false;
    let mut steps = Vec::new();
    let mut step_number = 0u8;
    let mut segment_builder = SegmentBuilder::new();

    if !tools.is_empty() {
        let mut tried_skill_only_tools = false;
        let mut skill_cache = skills::SkillRunCache::default();
        let mut round = 0u32;
        loop {
            round = round.saturating_add(1);
            step_number = step_number.saturating_add(1);
            if !host.is_generation_active(&config.conversation_id, config.generation) {
                host.emit_stream_done(
                    &config.conversation_id,
                    &config.run_id,
                    &config.message_id,
                    "cancelled",
                    "",
                );
                return Err("cancelled".to_string());
            }

            let prepared = prepare_agent_step(PrepareStepInput {
                step_number,
                previous_steps: &steps,
                runtime_messages: &runtime_messages,
                tools: &tools,
                phase: AgentPhase::ToolLoop,
            });
            let planning_reasoning_segment = segment_builder.reserve(
                ChatMessageSegmentKind::Reasoning,
                ChatMessageSegmentPhase::ToolLoop,
                Some(step_number),
                Some(round),
                &format!("step_{step_number}_reasoning"),
            );
            let planning_text_segment = segment_builder.reserve(
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
                segment_builder.next_order(),
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
                            return Err("cancelled".to_string());
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
                        return Ok(tool_planning_failed_run_result(
                            host,
                            &config,
                            segment_builder,
                            planning_text_segment.clone(),
                            planning_tool_drafts,
                            &planning_reasoning_parts,
                            generated_api_messages,
                            steps,
                            err.to_string(),
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
                    planning_final_already_streamed = step.streamed;
                    step.message
                }
                Err(err) if is_tools_unsupported_error(&err) => {
                    let skill_only: Vec<ChatToolDefinition> = tools
                        .iter()
                        .filter(|tool| tool.source == "skill")
                        .cloned()
                        .collect();
                    if !tried_skill_only_tools
                        && skill_only.len() < tools.len()
                        && !skill_only.is_empty()
                    {
                        eprintln!(
                            "Chat provider {} rejected tools; retrying with skill-native tools only",
                            config.provider.id
                        );
                        tools = skill_only;
                        tried_skill_only_tools = true;
                        continue;
                    }
                    eprintln!(
                        "Chat provider {} rejected tools; falling back to plain chat",
                        config.provider.id
                    );
                    provider_tools_unsupported = true;
                    steps.push(AgentStepResult {
                        step_number,
                        phase: AgentPhase::ToolLoop,
                        response_messages: Vec::new(),
                        tool_records: Vec::new(),
                        segments: Vec::new(),
                        streamed: false,
                        stop_reason: Some(AgentStopReason::ProviderToolsUnsupported),
                    });
                    break;
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
                    if planning_final_already_streamed {
                        host.emit_stream_delta(
                            &config.conversation_id,
                            &config.run_id,
                            &config.message_id,
                            "",
                            None,
                            Some(&segment),
                        );
                    }
                    if let Some(segment) =
                        segment_builder.append_text_from_template(&segment, response)
                    {
                        step_segments.push(segment);
                    }
                }
                if let Some(reasoning) = extract_reasoning_content(&message) {
                    let mut segment = planning_reasoning_segment.clone();
                    segment.phase = ChatMessageSegmentPhase::Plain;
                    if planning_final_already_streamed {
                        host.emit_stream_delta(
                            &config.conversation_id,
                            &config.run_id,
                            &config.message_id,
                            "",
                            None,
                            Some(&segment),
                        );
                    }
                    if let Some(segment) =
                        segment_builder.append_text_from_template(&segment, reasoning)
                    {
                        step_segments.push(segment);
                    }
                }
                planning_final_message = Some(message.clone());
                steps.push(AgentStepResult {
                    step_number,
                    phase: AgentPhase::ToolLoop,
                    response_messages: vec![message],
                    tool_records: Vec::new(),
                    segments: step_segments,
                    streamed: planning_final_already_streamed,
                    stop_reason: Some(AgentStopReason::Natural),
                });
                break;
            }
            planning_final_already_streamed = false;
            let planning_text =
                sanitize_assistant_text_response(&assistant_content_from_api_message(&message));
            let mut step_segments = Vec::new();
            if !planning_text.trim().is_empty() {
                if let Some(segment) =
                    segment_builder.append_text_from_template(&planning_text_segment, planning_text)
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
                if let Some(segment) = segment_builder
                    .append_text_from_template(&planning_reasoning_segment, reasoning.clone())
                {
                    step_segments.push(segment);
                }
                planning_reasoning_parts.push(reasoning);
            }

            let visible_tool_calls =
                visible_tool_segment_calls(&tools, &blocked_tool_calls, &tool_calls);
            let draft_tool_segments = planning_tool_drafts.segments();
            let tool_segments = if draft_tool_segments.is_empty() {
                let tool_segments = segment_builder.append_tool_calls(
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
                segment_builder.append_existing_segments(draft_tool_segments)
            };
            step_segments.extend(tool_segments);

            let assistant_message = assistant_api_message_for_tool_calls(&message, &tool_calls);
            runtime_messages.push(assistant_message);
            generated_api_messages.push(runtime_messages.last().cloned().unwrap_or(Value::Null));
            let mut step_response_messages =
                vec![runtime_messages.last().cloned().unwrap_or(Value::Null)];
            let round_result = execute_tool_round(
                host,
                executor,
                &config.settings,
                ToolRoundContext {
                    conversation_id: &config.conversation_id,
                    run_id: &config.run_id,
                    message_id: &config.message_id,
                    generation: config.generation,
                    round,
                },
                &tools,
                &blocked_tool_calls,
                tool_calls,
                &mut skill_cache,
            )
            .await;
            let round_cancelled = round_result.cancelled;
            runtime_messages.extend(round_result.response_messages.iter().cloned());
            generated_api_messages.extend(round_result.response_messages.iter().cloned());
            step_response_messages.extend(round_result.response_messages);
            let step_tool_records = round_result.tool_records;
            tool_records.extend(step_tool_records.iter().cloned());
            steps.push(AgentStepResult {
                step_number,
                phase: AgentPhase::ToolLoop,
                response_messages: step_response_messages,
                tool_records: step_tool_records,
                segments: step_segments,
                streamed: config.stream_enabled,
                stop_reason: if round_cancelled {
                    Some(AgentStopReason::Cancelled)
                } else {
                    None
                },
            });
            if round_cancelled {
                host.emit_stream_done(
                    &config.conversation_id,
                    &config.run_id,
                    &config.message_id,
                    "cancelled",
                    "",
                );
                return Ok(cancelled_tool_round_run_result(
                    &config.language,
                    &planning_reasoning_parts,
                    tool_records,
                    segment_builder.all(),
                    generated_api_messages,
                    steps,
                ));
            }
            if tool_round_limit_reached(config.effective_chat_tools.max_tool_rounds, round) {
                runtime_messages.push(step_limit_system_message());
                if let Some(last_step) = steps.last_mut() {
                    last_step.stop_reason = Some(AgentStopReason::StepLimit);
                }
                break;
            }
        }
    }

    if provider_tools_unsupported {
        patch_system_message(
            &mut runtime_messages,
            &config.provider_tools_fallback_system_prompt,
        );
    }

    if let Some(message) = planning_final_message {
        let (response, reasoning) =
            final_response_from_planning_message(&message, &planning_reasoning_parts)?;
        if !planning_final_already_streamed {
            host.emit_stream_delta(
                &config.conversation_id,
                &config.run_id,
                &config.message_id,
                &response,
                None,
                None,
            );
            host.emit_stream_done(
                &config.conversation_id,
                &config.run_id,
                &config.message_id,
                "done",
                &response,
            );
        }
        if !generated_api_messages.is_empty() {
            generated_api_messages.push(message);
        }
        return Ok(AgentRunResult {
            content: response,
            reasoning,
            tool_records,
            segments: segment_builder.all(),
            api_messages: generated_api_messages,
            steps,
            stream_outcome: "completed".to_string(),
        });
    }

    step_number = step_number.saturating_add(1);
    let phase = if tool_records.is_empty() && !provider_tools_unsupported {
        AgentPhase::Plain
    } else {
        AgentPhase::Synthesis
    };
    let prepared = prepare_agent_step(PrepareStepInput {
        step_number,
        previous_steps: &steps,
        runtime_messages: &runtime_messages,
        tools: &[],
        phase,
    });
    let synthesis_stream_policy = if tool_records.is_empty() {
        AgentStreamPolicy::SynthesisAlwaysDone
    } else {
        AgentStreamPolicy::SynthesisDeferEmpty
    };
    let response_phase = segment_phase_for_agent_phase(phase);
    let response_reasoning_segment = segment_builder.reserve(
        ChatMessageSegmentKind::Reasoning,
        response_phase.clone(),
        Some(step_number),
        None,
        &format!("step_{step_number}_reasoning"),
    );
    let response_segment = segment_builder.reserve(
        ChatMessageSegmentKind::Text,
        response_phase.clone(),
        Some(step_number),
        None,
        &format!("step_{step_number}_text"),
    );

    let (response, reasoning, response_reasoning) = if config.stream_enabled {
        let stream = stream_scoped_chat_completion_inner(
            config.state,
            host,
            &config.provider,
            &config.model,
            prepared.runtime_messages,
            None,
            config.retry_attempts,
            config.thinking_enabled,
            config.max_output_tokens,
            &config.conversation_id,
            &config.run_id,
            &config.message_id,
            config.generation,
            "Chat stream",
            synthesis_stream_policy,
            Some(response_segment.clone()),
            Some(response_reasoning_segment.clone()),
            None,
        )
        .await
        .map_err(|err| {
            if tool_records.is_empty() {
                err.to_string()
            } else {
                eprintln!(
                    "Chat synthesis stream failed after tool records; preserving tool results with fallback: {}",
                    err
                );
                String::new()
            }
        });
        let stream = match stream {
            Ok(stream) => stream,
            Err(err) if err.is_empty() && !tool_records.is_empty() => {
                let fallback = synthesis_failed_fallback_response(&config.language);
                host.emit_stream_delta(
                    &config.conversation_id,
                    &config.run_id,
                    &config.message_id,
                    &fallback,
                    None,
                    Some(&response_segment),
                );
                host.emit_stream_done(
                    &config.conversation_id,
                    &config.run_id,
                    &config.message_id,
                    "done",
                    &fallback,
                );
                if !generated_api_messages.is_empty() {
                    generated_api_messages.push(final_assistant_api_message(&fallback, None));
                }
                return Ok(AgentRunResult {
                    content: fallback.clone(),
                    reasoning: merge_reasoning(&planning_reasoning_parts, None),
                    tool_records,
                    segments: {
                        segment_builder.append_text_from_template(&response_segment, fallback);
                        segment_builder.all()
                    },
                    api_messages: generated_api_messages,
                    steps,
                    stream_outcome: "error".to_string(),
                });
            }
            Err(err) => return Err(err),
        };
        if stream.cancelled {
            if !tool_records.is_empty() {
                let stored_content = if stream.content.trim().is_empty() {
                    stopped_generation_content(&config.language)
                } else {
                    stream.content.clone()
                };
                let final_reasoning_for_api = stream.reasoning.clone();
                let reasoning = merge_reasoning(&planning_reasoning_parts, stream.reasoning);
                if !generated_api_messages.is_empty() {
                    generated_api_messages.push(final_assistant_api_message(
                        &stored_content,
                        final_reasoning_for_api.as_deref(),
                    ));
                }
                return Ok(AgentRunResult {
                    content: stored_content.clone(),
                    reasoning,
                    tool_records,
                    segments: {
                        segment_builder
                            .append_text_from_template(&response_segment, stored_content);
                        segment_builder.all()
                    },
                    api_messages: generated_api_messages,
                    steps,
                    stream_outcome: "cancelled".to_string(),
                });
            }
            return Err("cancelled".to_string());
        }
        let final_reasoning_for_api = stream.reasoning.clone();
        let reasoning = merge_reasoning(&planning_reasoning_parts, stream.reasoning.clone());
        let response = sanitize_assistant_text_response(&stream.content);
        if response.trim().is_empty() {
            if !tool_records.is_empty() {
                log_empty_synthesis_output(&config, phase, &stream, tool_records.len());
                let fallback = empty_synthesis_fallback_response(&config.language);
                host.emit_stream_delta(
                    &config.conversation_id,
                    &config.run_id,
                    &config.message_id,
                    &fallback,
                    None,
                    Some(&response_segment),
                );
                host.emit_stream_done(
                    &config.conversation_id,
                    &config.run_id,
                    &config.message_id,
                    "done",
                    &fallback,
                );
                if !generated_api_messages.is_empty() {
                    generated_api_messages.push(final_assistant_api_message(
                        &fallback,
                        final_reasoning_for_api.as_deref(),
                    ));
                }
                (fallback, reasoning, final_reasoning_for_api)
            } else {
                return Err(empty_assistant_response_error("Chat stream"));
            }
        } else {
            if !generated_api_messages.is_empty() {
                generated_api_messages.push(final_assistant_api_message(
                    &response,
                    final_reasoning_for_api.as_deref(),
                ));
            }
            (response, reasoning, final_reasoning_for_api)
        }
    } else {
        let message_result = tokio::select! {
            result = call_chat_completion_message(
                config.state,
                &config.provider,
                &config.model,
                runtime_messages,
                None,
                config.retry_attempts,
                config.thinking_enabled,
                config.max_output_tokens,
                &config.conversation_id,
                &config.message_id,
                "Chat API",
            ) => result,
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
        };
        let message = match message_result {
            Ok(message) => message,
            Err(err) if !tool_records.is_empty() => {
                eprintln!(
                    "Chat synthesis request failed after tool records; preserving tool results with fallback: {}",
                    err
                );
                let fallback = synthesis_failed_fallback_response(&config.language);
                host.emit_stream_delta(
                    &config.conversation_id,
                    &config.run_id,
                    &config.message_id,
                    &fallback,
                    None,
                    Some(&response_segment),
                );
                host.emit_stream_done(
                    &config.conversation_id,
                    &config.run_id,
                    &config.message_id,
                    "done",
                    &fallback,
                );
                if !generated_api_messages.is_empty() {
                    generated_api_messages.push(final_assistant_api_message(&fallback, None));
                }
                return Ok(AgentRunResult {
                    content: fallback.clone(),
                    reasoning: merge_reasoning(&planning_reasoning_parts, None),
                    tool_records,
                    segments: {
                        segment_builder.append_text_from_template(&response_segment, fallback);
                        segment_builder.all()
                    },
                    api_messages: generated_api_messages,
                    steps,
                    stream_outcome: "error".to_string(),
                });
            }
            Err(err) => return Err(err),
        };
        let response = sanitize_assistant_text_response(
            message
                .get("content")
                .and_then(|content| content.as_str())
                .unwrap_or_default(),
        );
        let reasoning = merge_reasoning(
            &planning_reasoning_parts,
            extract_reasoning_content(&message),
        );
        let response_reasoning = extract_reasoning_content(&message);
        if response.trim().is_empty() && !tool_records.is_empty() {
            eprintln!(
                "Chat agent empty synthesis fallback: conversation_id={} run_id={} provider_id={} model={} phase={:?} stream=false tool_records={} finish_reason={}",
                config.conversation_id,
                config.run_id,
                config.provider.id,
                config.model,
                phase,
                tool_records.len(),
                message
                    .get("finish_reason")
                    .and_then(|value| value.as_str())
                    .unwrap_or("unknown"),
            );
            let fallback = empty_synthesis_fallback_response(&config.language);
            host.emit_stream_delta(
                &config.conversation_id,
                &config.run_id,
                &config.message_id,
                &fallback,
                None,
                Some(&response_segment),
            );
            host.emit_stream_done(
                &config.conversation_id,
                &config.run_id,
                &config.message_id,
                "done",
                &fallback,
            );
            if !generated_api_messages.is_empty() {
                generated_api_messages.push(final_assistant_api_message(
                    &fallback,
                    extract_reasoning_content(&message).as_deref(),
                ));
            }
            (fallback, reasoning, response_reasoning)
        } else if response.trim().is_empty() {
            host.emit_stream_done(
                &config.conversation_id,
                &config.run_id,
                &config.message_id,
                "error",
                "",
            );
            return Err(empty_assistant_response_error("Chat API"));
        } else {
            host.emit_stream_delta(
                &config.conversation_id,
                &config.run_id,
                &config.message_id,
                &response,
                None,
                Some(&response_segment),
            );
            host.emit_stream_done(
                &config.conversation_id,
                &config.run_id,
                &config.message_id,
                "done",
                &response,
            );
            if !generated_api_messages.is_empty() {
                generated_api_messages.push(message);
            }
            (response, reasoning, response_reasoning)
        }
    };
    let mut final_step_segments = Vec::new();
    if !response.trim().is_empty() {
        if let Some(segment) =
            segment_builder.append_text_from_template(&response_segment, response.clone())
        {
            final_step_segments.push(segment);
        }
    }
    if let Some(reasoning_part) = response_reasoning
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if let Some(segment) =
            segment_builder.append_text_from_template(&response_reasoning_segment, reasoning_part)
        {
            final_step_segments.push(segment);
        }
    }

    steps.push(AgentStepResult {
        step_number,
        phase,
        response_messages: Vec::new(),
        tool_records: Vec::new(),
        segments: final_step_segments,
        streamed: config.stream_enabled,
        stop_reason: Some(AgentStopReason::Natural),
    });

    Ok(AgentRunResult {
        content: response,
        reasoning,
        tool_records,
        segments: segment_builder.all(),
        api_messages: generated_api_messages,
        steps,
        stream_outcome: "completed".to_string(),
    })
}

fn tool_round_limit_reached(max_tool_rounds: Option<u32>, round: u32) -> bool {
    max_tool_rounds.is_some_and(|limit| round >= limit)
}

async fn execute_tool_round(
    host: &dyn AgentHost,
    executor: &dyn ToolExecutor,
    settings: &Settings,
    ctx: ToolRoundContext<'_>,
    tools: &[ChatToolDefinition],
    blocked_tool_calls: &[ChatToolDefinition],
    tool_calls: Vec<PendingToolCall>,
    skill_cache: &mut skills::SkillRunCache,
) -> ToolRoundResult {
    let mut response_messages = Vec::new();
    let mut tool_records = Vec::new();
    let mut parallel_batch: Vec<ExecutableToolCall<'_>> = Vec::new();
    let mut cancelled = false;

    let mut tool_calls = tool_calls.into_iter();
    while let Some(tool_call) = tool_calls.next() {
        let Some(tool) = match_tool_call(tools, &tool_call.function_name) else {
            if flush_parallel_batch(
                &mut parallel_batch,
                &mut response_messages,
                &mut tool_records,
                host,
                executor,
                settings,
                &ctx,
            )
            .await
            {
                cancelled = true;
                push_cancelled_tool_call(
                    host,
                    &ctx,
                    tools,
                    tool_call,
                    &mut response_messages,
                    &mut tool_records,
                );
                push_cancelled_tool_calls(
                    host,
                    &ctx,
                    tools,
                    tool_calls,
                    &mut response_messages,
                    &mut tool_records,
                );
                break;
            }
            let result = unknown_or_disabled_tool_result(host, &ctx, blocked_tool_calls, tool_call);
            push_tool_execution_result(result, &mut response_messages, &mut tool_records);
            continue;
        };

        if let Some(error) = tool_call.arguments_parse_error.clone() {
            if flush_parallel_batch(
                &mut parallel_batch,
                &mut response_messages,
                &mut tool_records,
                host,
                executor,
                settings,
                &ctx,
            )
            .await
            {
                cancelled = true;
                push_cancelled_tool_call(
                    host,
                    &ctx,
                    tools,
                    tool_call,
                    &mut response_messages,
                    &mut tool_records,
                );
                push_cancelled_tool_calls(
                    host,
                    &ctx,
                    tools,
                    tool_calls,
                    &mut response_messages,
                    &mut tool_records,
                );
                break;
            }
            let result = invalid_tool_arguments_result(host, &ctx, &tool_call, tool, error);
            push_tool_execution_result(result, &mut response_messages, &mut tool_records);
            continue;
        }

        if tool_call_parallel_eligible(settings, tool) {
            parallel_batch.push(ExecutableToolCall {
                call: tool_call,
                tool,
            });
            if parallel_batch.len() >= MAX_PARALLEL_TOOL_CALLS_PER_ROUND {
                if flush_parallel_batch(
                    &mut parallel_batch,
                    &mut response_messages,
                    &mut tool_records,
                    host,
                    executor,
                    settings,
                    &ctx,
                )
                .await
                {
                    cancelled = true;
                    push_cancelled_tool_calls(
                        host,
                        &ctx,
                        tools,
                        tool_calls,
                        &mut response_messages,
                        &mut tool_records,
                    );
                    break;
                }
            }
            continue;
        }

        if flush_parallel_batch(
            &mut parallel_batch,
            &mut response_messages,
            &mut tool_records,
            host,
            executor,
            settings,
            &ctx,
        )
        .await
        {
            cancelled = true;
            push_cancelled_tool_call(
                host,
                &ctx,
                tools,
                tool_call,
                &mut response_messages,
                &mut tool_records,
            );
            push_cancelled_tool_calls(
                host,
                &ctx,
                tools,
                tool_calls,
                &mut response_messages,
                &mut tool_records,
            );
            break;
        }
        let result =
            execute_serial_tool_call(host, executor, settings, &ctx, tool, tool_call, skill_cache)
                .await;
        if push_tool_execution_result(result, &mut response_messages, &mut tool_records) {
            cancelled = true;
            push_cancelled_tool_calls(
                host,
                &ctx,
                tools,
                tool_calls,
                &mut response_messages,
                &mut tool_records,
            );
            break;
        }
    }

    if !cancelled {
        cancelled = flush_parallel_batch(
            &mut parallel_batch,
            &mut response_messages,
            &mut tool_records,
            host,
            executor,
            settings,
            &ctx,
        )
        .await;
    }

    ToolRoundResult {
        response_messages,
        tool_records,
        cancelled,
    }
}

fn push_tool_execution_result(
    result: ToolExecutionResult,
    response_messages: &mut Vec<Value>,
    tool_records: &mut Vec<ToolCallRecord>,
) -> bool {
    let cancelled = result.cancelled;
    if let Some(record) = result.record {
        tool_records.push(record);
    }
    response_messages.push(result.response_message);
    cancelled
}

async fn flush_parallel_batch(
    batch: &mut Vec<ExecutableToolCall<'_>>,
    response_messages: &mut Vec<Value>,
    tool_records: &mut Vec<ToolCallRecord>,
    host: &dyn AgentHost,
    executor: &dyn ToolExecutor,
    settings: &Settings,
    ctx: &ToolRoundContext<'_>,
) -> bool {
    if batch.is_empty() {
        return false;
    }

    let mut cancelled = false;
    while !batch.is_empty() {
        let limit = batch.len().min(MAX_PARALLEL_TOOL_CALLS_PER_ROUND);
        let mut chunk = batch.drain(..limit).collect::<Vec<_>>();
        let results = execute_parallel_chunk(host, executor, settings, ctx, &mut chunk).await;
        for result in results {
            cancelled |= push_tool_execution_result(result, response_messages, tool_records);
        }
        if cancelled {
            for item in batch.drain(..) {
                let result = cancelled_tool_result(host, ctx, &item.call, Some(item.tool));
                push_tool_execution_result(result, response_messages, tool_records);
            }
            break;
        }
    }
    cancelled
}

async fn execute_parallel_chunk(
    host: &dyn AgentHost,
    executor: &dyn ToolExecutor,
    settings: &Settings,
    ctx: &ToolRoundContext<'_>,
    chunk: &mut [ExecutableToolCall<'_>],
) -> Vec<ToolExecutionResult> {
    match chunk.len() {
        0 => Vec::new(),
        1 => {
            let item = &chunk[0];
            vec![execute_parallel_tool_call(host, executor, settings, ctx, item).await]
        }
        2 => {
            let (a, rest) = chunk.split_at(1);
            let (b, _) = rest.split_at(1);
            let (ra, rb) = tokio::join!(
                execute_parallel_tool_call(host, executor, settings, ctx, &a[0]),
                execute_parallel_tool_call(host, executor, settings, ctx, &b[0]),
            );
            vec![ra, rb]
        }
        3 => {
            let (a, rest) = chunk.split_at(1);
            let (b, rest) = rest.split_at(1);
            let (c, _) = rest.split_at(1);
            let (ra, rb, rc) = tokio::join!(
                execute_parallel_tool_call(host, executor, settings, ctx, &a[0]),
                execute_parallel_tool_call(host, executor, settings, ctx, &b[0]),
                execute_parallel_tool_call(host, executor, settings, ctx, &c[0]),
            );
            vec![ra, rb, rc]
        }
        _ => {
            let (a, rest) = chunk.split_at(1);
            let (b, rest) = rest.split_at(1);
            let (c, rest) = rest.split_at(1);
            let (d, _) = rest.split_at(1);
            let (ra, rb, rc, rd) = tokio::join!(
                execute_parallel_tool_call(host, executor, settings, ctx, &a[0]),
                execute_parallel_tool_call(host, executor, settings, ctx, &b[0]),
                execute_parallel_tool_call(host, executor, settings, ctx, &c[0]),
                execute_parallel_tool_call(host, executor, settings, ctx, &d[0]),
            );
            vec![ra, rb, rc, rd]
        }
    }
}

async fn execute_parallel_tool_call(
    host: &dyn AgentHost,
    executor: &dyn ToolExecutor,
    settings: &Settings,
    ctx: &ToolRoundContext<'_>,
    item: &ExecutableToolCall<'_>,
) -> ToolExecutionResult {
    execute_tool_call_result(
        host,
        executor,
        settings,
        ctx,
        item.tool,
        item.call.clone(),
        None,
    )
    .await
}

async fn execute_serial_tool_call(
    host: &dyn AgentHost,
    executor: &dyn ToolExecutor,
    settings: &Settings,
    ctx: &ToolRoundContext<'_>,
    tool: &ChatToolDefinition,
    tool_call: PendingToolCall,
    skill_cache: &mut skills::SkillRunCache,
) -> ToolExecutionResult {
    execute_tool_call_result(
        host,
        executor,
        settings,
        ctx,
        tool,
        tool_call,
        Some(skill_cache),
    )
    .await
}

async fn execute_tool_call_result(
    host: &dyn AgentHost,
    executor: &dyn ToolExecutor,
    settings: &Settings,
    round_ctx: &ToolRoundContext<'_>,
    tool: &ChatToolDefinition,
    tool_call: PendingToolCall,
    skill_cache: Option<&mut skills::SkillRunCache>,
) -> ToolExecutionResult {
    let tool_call_id = tool_call.id.clone();
    let execution_ctx = ToolExecutionContext {
        conversation_id: round_ctx.conversation_id,
        run_id: round_ctx.run_id,
        message_id: round_ctx.message_id,
        generation: round_ctx.generation,
        round: round_ctx.round,
    };
    let (record, tool_content) = execute_tool_call(
        host,
        executor,
        settings,
        &execution_ctx,
        tool,
        tool_call,
        skill_cache,
    )
    .await;
    let cancelled = matches!(record.status, ToolCallStatus::Cancelled);
    ToolExecutionResult {
        response_message: tool_message(tool_call_id, tool_content),
        record: Some(record),
        cancelled,
    }
}

fn push_cancelled_tool_calls(
    host: &dyn AgentHost,
    ctx: &ToolRoundContext<'_>,
    tools: &[ChatToolDefinition],
    tool_calls: impl IntoIterator<Item = PendingToolCall>,
    response_messages: &mut Vec<Value>,
    tool_records: &mut Vec<ToolCallRecord>,
) {
    for tool_call in tool_calls {
        push_cancelled_tool_call(host, ctx, tools, tool_call, response_messages, tool_records);
    }
}

fn push_cancelled_tool_call(
    host: &dyn AgentHost,
    ctx: &ToolRoundContext<'_>,
    tools: &[ChatToolDefinition],
    tool_call: PendingToolCall,
    response_messages: &mut Vec<Value>,
    tool_records: &mut Vec<ToolCallRecord>,
) {
    let tool = match_tool_call(tools, &tool_call.function_name);
    let result = cancelled_tool_result(host, ctx, &tool_call, tool);
    push_tool_execution_result(result, response_messages, tool_records);
}

fn cancelled_tool_result(
    host: &dyn AgentHost,
    ctx: &ToolRoundContext<'_>,
    tool_call: &PendingToolCall,
    tool: Option<&ChatToolDefinition>,
) -> ToolExecutionResult {
    let now = chrono::Local::now().timestamp();
    let record = ToolCallRecord {
        id: tool_call.id.clone(),
        name: tool
            .map(|tool| tool.name.clone())
            .unwrap_or_else(|| tool_call.function_name.clone()),
        source: tool
            .map(|tool| tool.source.clone())
            .unwrap_or_else(|| "unknown".to_string()),
        server_id: tool.and_then(|tool| tool.server_id.clone()),
        arguments: tool_call.arguments_raw.clone(),
        status: ToolCallStatus::Cancelled,
        result_preview: None,
        error: Some("Tool call cancelled".to_string()),
        duration_ms: Some(0),
        started_at: Some(now),
        completed_at: Some(now),
        round: ctx.round,
        sensitive: tool.map(|tool| tool.sensitive).unwrap_or(false),
        artifacts: Vec::new(),
        trace_id: Some(ctx.run_id.to_string()),
        span_id: Some(tool_span_id(ctx, &tool_call.id)),
        structured_content: None,
    };
    host.emit_tool_record(ctx.conversation_id, ctx.run_id, ctx.message_id, &record);
    ToolExecutionResult {
        response_message: tool_message(tool_call.id.clone(), "Tool call cancelled"),
        record: Some(record),
        cancelled: true,
    }
}

fn unknown_or_disabled_tool_result(
    host: &dyn AgentHost,
    ctx: &ToolRoundContext<'_>,
    blocked_tool_calls: &[ChatToolDefinition],
    tool_call: PendingToolCall,
) -> ToolExecutionResult {
    if let Some(tool) = match_tool_call(blocked_tool_calls, &tool_call.function_name) {
        let error = format!(
            "Tool `{}` is blocked in Plan mode. Switch to Act / execute the plan before using side-effecting tools.",
            tool.openai_tool_name()
        );
        let mut record = skipped_tool_record(ctx, &tool_call, tool, error.clone());
        attach_tool_trace(ctx, &mut record);
        host.emit_tool_record(ctx.conversation_id, ctx.run_id, ctx.message_id, &record);
        return ToolExecutionResult {
            response_message: tool_message(tool_call.id, error),
            record: Some(record),
            cancelled: false,
        };
    }

    let disabled = disabled_tool_content(&tool_call);
    let record = if disabled.is_none() {
        let error = format!("Unknown tool requested: {}", tool_call.function_name);
        let mut record = unknown_tool_record(&tool_call, ctx.round, error);
        attach_tool_trace(ctx, &mut record);
        host.emit_tool_record(ctx.conversation_id, ctx.run_id, ctx.message_id, &record);
        Some(record)
    } else {
        None
    };
    let content =
        disabled.unwrap_or_else(|| format!("Unknown tool requested: {}", tool_call.function_name));
    ToolExecutionResult {
        response_message: tool_message(tool_call.id, content),
        record,
        cancelled: false,
    }
}

fn skipped_tool_record(
    ctx: &ToolRoundContext<'_>,
    call: &PendingToolCall,
    tool: &ChatToolDefinition,
    error: String,
) -> ToolCallRecord {
    let now = chrono::Local::now().timestamp();
    ToolCallRecord {
        id: call.id.clone(),
        name: tool.name.clone(),
        source: tool.source.clone(),
        server_id: tool.server_id.clone(),
        arguments: call.arguments_raw.clone(),
        status: ToolCallStatus::Skipped,
        result_preview: None,
        error: Some(error),
        duration_ms: Some(0),
        started_at: Some(now),
        completed_at: Some(now),
        round: ctx.round,
        sensitive: tool.sensitive,
        artifacts: Vec::new(),
        trace_id: None,
        span_id: None,
        structured_content: None,
    }
}

fn invalid_tool_arguments_result(
    host: &dyn AgentHost,
    ctx: &ToolRoundContext<'_>,
    tool_call: &PendingToolCall,
    tool: &ChatToolDefinition,
    error: String,
) -> ToolExecutionResult {
    let mut record = invalid_tool_arguments_record(tool_call, tool, ctx.round, error);
    attach_tool_trace(ctx, &mut record);
    host.emit_tool_record(ctx.conversation_id, ctx.run_id, ctx.message_id, &record);
    ToolExecutionResult {
        response_message: tool_message(
            tool_call.id.clone(),
            "Tool arguments JSON is invalid or incomplete. Retry this tool call with a compact, valid JSON object for arguments.",
        ),
        record: Some(record),
        cancelled: false,
    }
}

fn attach_tool_trace(ctx: &ToolRoundContext<'_>, record: &mut ToolCallRecord) {
    record.trace_id = Some(ctx.run_id.to_string());
    record.span_id = Some(tool_span_id(ctx, &record.id));
}

fn tool_span_id(ctx: &ToolRoundContext<'_>, tool_call_id: &str) -> String {
    format!("tool_round_{}_{}", ctx.round, tool_call_id)
}

fn tool_message(tool_call_id: String, content: impl Into<String>) -> Value {
    serde_json::json!({
        "role": "tool",
        "tool_call_id": tool_call_id,
        "content": content.into(),
    })
}

fn tool_call_parallel_eligible(settings: &Settings, tool: &ChatToolDefinition) -> bool {
    if tool_requires_approval(settings, tool) {
        return false;
    }
    if tool.source == "native" {
        return matches!(
            tool.name.as_str(),
            "web_search"
                | "web_fetch"
                | "read_file"
                | "list_dir"
                | "search_files"
                | "glob_files"
                | "stat_path"
        );
    }
    tool.source == "mcp" && tool.is_read_only_tool()
}

async fn call_chat_completion_message(
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
async fn stream_scoped_chat_completion_inner(
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

fn empty_synthesis_fallback_response(language: &str) -> String {
    if language.starts_with("zh") {
        "工具调用已经完成，但模型没有返回最终总结。上方工具结果已保存在本轮回复中，你可以继续追问，或让我重新生成总结。".to_string()
    } else {
        "The tool calls completed, but the model did not return a final summary. The tool results above were saved with this reply; you can continue from them or regenerate the summary.".to_string()
    }
}

fn synthesis_failed_fallback_response(language: &str) -> String {
    if language.starts_with("zh") {
        "工具调用已经完成，但最终总结生成失败。上方工具结果已保存在本轮回复中，你可以继续追问，或让我重新生成总结。".to_string()
    } else {
        "The tool calls completed, but final summary generation failed. The tool results above were saved with this reply; you can continue from them or regenerate the summary.".to_string()
    }
}

fn tool_planning_failed_fallback_response(language: &str) -> String {
    if language.starts_with("zh") {
        "工具调用参数生成失败，这一步还没有真正执行写入。主对话已保留，你可以让我缩小范围、改用补丁，或重新生成。".to_string()
    } else {
        "Tool-call argument generation failed before the write actually ran. This conversation was preserved; you can ask me to narrow the scope, use a patch, or regenerate.".to_string()
    }
}

fn stopped_generation_content(language: &str) -> String {
    if language.starts_with("zh") {
        "已停止生成。".to_string()
    } else {
        "Generation stopped.".to_string()
    }
}

fn tool_planning_failed_run_result(
    host: &dyn AgentHost,
    config: &AgentRunConfig<'_>,
    mut segment_builder: SegmentBuilder,
    planning_text_segment: ChatMessageSegment,
    tool_draft_tracker: ToolCallDraftTracker,
    planning_reasoning_parts: &[String],
    mut generated_api_messages: Vec<Value>,
    mut steps: Vec<AgentStepResult>,
    error: String,
) -> AgentRunResult {
    let failed_records = tool_draft_tracker.mark_error(&error);
    for record in &failed_records {
        host.emit_tool_record(
            &config.conversation_id,
            &config.run_id,
            &config.message_id,
            record,
        );
    }

    let mut step_segments = segment_builder.append_existing_segments(tool_draft_tracker.segments());
    let content = tool_planning_failed_fallback_response(&config.language);
    let mut final_segment = planning_text_segment;
    final_segment.phase = ChatMessageSegmentPhase::Synthesis;
    final_segment.round = None;
    if let Some(segment) =
        segment_builder.append_text_from_template(&final_segment, content.clone())
    {
        step_segments.push(segment.clone());
        host.emit_stream_delta(
            &config.conversation_id,
            &config.run_id,
            &config.message_id,
            &content,
            None,
            Some(&segment),
        );
    } else {
        host.emit_stream_delta(
            &config.conversation_id,
            &config.run_id,
            &config.message_id,
            &content,
            None,
            None,
        );
    }
    host.emit_stream_done(
        &config.conversation_id,
        &config.run_id,
        &config.message_id,
        "done",
        &content,
    );
    generated_api_messages.push(final_assistant_api_message(&content, None));
    steps.push(AgentStepResult {
        step_number: final_segment.step_number.unwrap_or_default(),
        phase: AgentPhase::ToolLoop,
        response_messages: Vec::new(),
        tool_records: failed_records.clone(),
        segments: step_segments,
        streamed: config.stream_enabled,
        stop_reason: Some(AgentStopReason::Natural),
    });

    AgentRunResult {
        content,
        reasoning: merge_reasoning(planning_reasoning_parts, None),
        tool_records: failed_records,
        segments: segment_builder.all(),
        api_messages: generated_api_messages,
        steps,
        stream_outcome: "error".to_string(),
    }
}

fn cancelled_tool_round_run_result(
    language: &str,
    planning_reasoning_parts: &[String],
    tool_records: Vec<ToolCallRecord>,
    mut segments: Vec<ChatMessageSegment>,
    mut generated_api_messages: Vec<Value>,
    steps: Vec<AgentStepResult>,
) -> AgentRunResult {
    let stopped_content = stopped_generation_content(language);
    let next_order = segments
        .iter()
        .map(|segment| segment.order)
        .max()
        .unwrap_or(999)
        .saturating_add(1);
    segments.push(ChatMessageSegment {
        id: format!("seg_{}_cancelled_synthesis", next_order),
        kind: ChatMessageSegmentKind::Text,
        phase: ChatMessageSegmentPhase::Synthesis,
        order: next_order,
        step_number: None,
        round: None,
        text: Some(stopped_content.clone()),
        tool_call_id: None,
    });
    if !generated_api_messages.is_empty() {
        generated_api_messages.push(final_assistant_api_message(&stopped_content, None));
    }
    AgentRunResult {
        content: stopped_content,
        reasoning: merge_reasoning(planning_reasoning_parts, None),
        tool_records,
        segments,
        api_messages: generated_api_messages,
        steps,
        stream_outcome: "cancelled".to_string(),
    }
}

fn log_empty_synthesis_output(
    config: &AgentRunConfig<'_>,
    phase: AgentPhase,
    stream: &ChatStreamOutput,
    tool_record_count: usize,
) {
    eprintln!(
        "Chat agent empty synthesis fallback: conversation_id={} run_id={} provider_id={} model={} phase={:?} stream=true tool_records={} finish_reason={} raw_chars={} cleaned_chars={} reasoning_chars={} stream_tool_calls={}",
        config.conversation_id,
        config.run_id,
        config.provider.id,
        config.model,
        phase,
        tool_record_count,
        stream.finish_reason.as_deref().unwrap_or("unknown"),
        stream.raw_content.chars().count(),
        stream.content.chars().count(),
        stream.reasoning.as_deref().map(|value| value.chars().count()).unwrap_or(0),
        stream.tool_calls.len(),
    );
}

async fn generate_with_chat_provider(
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

async fn stream_with_chat_provider(
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

#[cfg(test)]
mod tests {
    use std::io::{Read as _, Write as _};
    use std::net::{TcpListener, TcpStream};
    use std::sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    };

    use tokio::time::{sleep, Duration};

    use super::*;
    use crate::chat::agent::types::AgentRunEntry;
    use crate::chat::model::{StreamPart, StreamSink as _};
    use crate::chat::types::ToolCallStatus;
    use crate::mcp::types::{
        native_read_file_tool, native_run_python_tool, native_web_fetch_tool,
        native_write_file_tool, McpToolCallResult,
    };
    use crate::settings::{ChatToolsConfig, ModelProvider};
    use crate::state::AppState;

    #[derive(Clone, Debug)]
    struct RecordedDelta {
        delta: String,
        reasoning_delta: Option<String>,
        segment: Option<ChatMessageSegment>,
    }

    #[derive(Default)]
    struct TestHost {
        records: Mutex<Vec<ToolCallRecord>>,
        deltas: Mutex<Vec<RecordedDelta>>,
        dones: Mutex<Vec<(String, String)>>,
        cancel_after: Option<Duration>,
        cancel_flag: Arc<AtomicBool>,
        cancel_on_first_text_delta: bool,
    }

    impl TestHost {
        fn cancelling_after(delay: Duration) -> Self {
            Self {
                cancel_after: Some(delay),
                ..Self::default()
            }
        }

        fn with_cancel_flag(cancel_flag: Arc<AtomicBool>) -> Self {
            Self {
                cancel_flag,
                ..Self::default()
            }
        }

        fn cancelling_on_first_text_delta() -> Self {
            Self {
                cancel_on_first_text_delta: true,
                ..Self::default()
            }
        }

        fn recorded_deltas(&self) -> Vec<RecordedDelta> {
            self.deltas
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .clone()
        }

        fn recorded_dones(&self) -> Vec<(String, String)> {
            self.dones
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .clone()
        }

        fn recorded_tool_records(&self) -> Vec<ToolCallRecord> {
            self.records
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .clone()
        }
    }

    impl AgentHost for TestHost {
        fn emit_stream_delta(
            &self,
            _conversation_id: &str,
            _run_id: &str,
            _message_id: &str,
            delta: &str,
            reasoning_delta: Option<&str>,
            segment: Option<&ChatMessageSegment>,
        ) {
            if self.cancel_on_first_text_delta && !delta.is_empty() {
                self.cancel_flag.store(true, Ordering::SeqCst);
            }
            self.deltas
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .push(RecordedDelta {
                    delta: delta.to_string(),
                    reasoning_delta: reasoning_delta.map(str::to_string),
                    segment: segment.cloned(),
                });
        }

        fn emit_stream_done(
            &self,
            _conversation_id: &str,
            _run_id: &str,
            _message_id: &str,
            reason: &str,
            full: &str,
        ) {
            self.dones
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .push((reason.to_string(), full.to_string()));
        }

        fn emit_tool_record(
            &self,
            _conversation_id: &str,
            _run_id: &str,
            _message_id: &str,
            record: &ToolCallRecord,
        ) {
            self.records
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .push(record.clone());
        }

        fn request_tool_approval<'a>(
            &'a self,
            _ctx: &'a ToolExecutionContext<'a>,
            _record: &'a ToolCallRecord,
        ) -> super::super::host::AgentHostFuture<'a, bool> {
            Box::pin(async { true })
        }

        fn request_user_response<'a>(
            &'a self,
            _ctx: &'a ToolExecutionContext<'a>,
            _record: &'a ToolCallRecord,
            _prompt: crate::chat::ask_user::AskUserPromptPayload,
        ) -> super::super::host::AgentHostFuture<'a, crate::chat::ask_user::AskUserResponseResult>
        {
            Box::pin(async { crate::chat::ask_user::skipped_response() })
        }

        fn is_generation_active(&self, _conversation_id: &str, _generation: u64) -> bool {
            !self.cancel_flag.load(Ordering::SeqCst)
        }

        fn wait_for_generation_inactive<'a>(
            &'a self,
            _conversation_id: &'a str,
            _generation: u64,
        ) -> super::super::host::AgentHostFuture<'a, ()> {
            let cancel_after = self.cancel_after;
            let cancel_flag = self.cancel_flag.clone();
            Box::pin(async move {
                let started = tokio::time::Instant::now();
                loop {
                    if cancel_flag.load(Ordering::SeqCst) {
                        return;
                    }
                    if let Some(delay) = cancel_after {
                        if started.elapsed() >= delay {
                            return;
                        }
                    }
                    sleep(Duration::from_millis(2)).await;
                }
            })
        }
    }

    #[derive(Default)]
    struct RecordingExecutor {
        active: AtomicUsize,
        max_active: AtomicUsize,
        events: Arc<Mutex<Vec<String>>>,
    }

    impl RecordingExecutor {
        fn max_active(&self) -> usize {
            self.max_active.load(Ordering::SeqCst)
        }

        fn events(&self) -> Vec<String> {
            self.events
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .clone()
        }
    }

    impl ToolExecutor for RecordingExecutor {
        fn call<'a>(
            &'a self,
            _ctx: &'a ToolExecutionContext<'a>,
            tool: &'a ChatToolDefinition,
            _arguments: Value,
            _skill_cache: Option<&'a mut skills::SkillRunCache>,
        ) -> super::super::execute::ToolExecutorFuture<'a> {
            let name = tool.name.clone();
            let events = self.events.clone();
            Box::pin(async move {
                let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
                self.max_active.fetch_max(active, Ordering::SeqCst);
                events
                    .lock()
                    .unwrap_or_else(|err| err.into_inner())
                    .push(format!("start:{name}"));
                sleep(Duration::from_millis(25)).await;
                events
                    .lock()
                    .unwrap_or_else(|err| err.into_inner())
                    .push(format!("finish:{name}"));
                self.active.fetch_sub(1, Ordering::SeqCst);
                Ok(McpToolCallResult {
                    content: format!("result:{name}"),
                    is_error: false,
                    raw: Value::Null,
                    artifacts: Vec::new(),
                    structured_content: None,
                })
            })
        }
    }

    /// Tool executor that succeeds immediately and flips the shared cancel flag while
    /// executing, so cancellation deterministically lands between the tool round and
    /// the synthesis request (the executor branch wins the `tokio::select!` because it
    /// completes synchronously on its first poll).
    struct CancelAfterToolExecutor {
        cancel_flag: Arc<AtomicBool>,
    }

    impl ToolExecutor for CancelAfterToolExecutor {
        fn call<'a>(
            &'a self,
            _ctx: &'a ToolExecutionContext<'a>,
            tool: &'a ChatToolDefinition,
            _arguments: Value,
            _skill_cache: Option<&'a mut skills::SkillRunCache>,
        ) -> super::super::execute::ToolExecutorFuture<'a> {
            let name = tool.name.clone();
            let cancel_flag = self.cancel_flag.clone();
            Box::pin(async move {
                cancel_flag.store(true, Ordering::SeqCst);
                Ok(McpToolCallResult {
                    content: format!("result:{name}"),
                    is_error: false,
                    raw: Value::Null,
                    artifacts: Vec::new(),
                    structured_content: None,
                })
            })
        }
    }

    /// Scripted HTTP mock for the OpenAI-compatible chat completions endpoint.
    /// Responses are served in connection-accept order; each response closes (or
    /// deliberately breaks) its connection so reqwest opens a fresh one per request.
    enum MockResponse {
        /// Complete JSON chat completion body.
        Json(String),
        /// SSE stream; each entry becomes one `data: <entry>` event.
        Sse(Vec<String>),
        /// Plain HTTP error status with a JSON body.
        Status(u16, String),
        /// Chunked SSE that drops the connection without the chunked terminator,
        /// producing a reqwest decode error (StreamReadInterrupted).
        SseInterrupt(Vec<String>),
        /// Chunked SSE that writes the given events then keeps the connection open,
        /// simulating a hung provider so cancellation paths can win the select.
        SseThenHang(Vec<String>),
    }

    struct MockModelServer {
        base_url: String,
    }

    impl MockModelServer {
        fn start(responses: Vec<MockResponse>) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock model server");
            let addr = listener.local_addr().expect("mock model server addr");
            std::thread::spawn(move || {
                for response in responses {
                    let Ok((mut stream, _)) = listener.accept() else {
                        return;
                    };
                    stream
                        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                        .ok();
                    if read_http_request(&mut stream).is_err() {
                        continue;
                    }
                    serve_mock_response(stream, response);
                }
            });
            Self {
                base_url: format!("http://{addr}/v1"),
            }
        }
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
            if let Some(pos) = buf.windows(4).position(|window| window == b"\r\n\r\n") {
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

    fn sse_body(events: &[String]) -> String {
        events
            .iter()
            .map(|event| format!("data: {event}\n\n"))
            .collect()
    }

    fn serve_mock_response(mut stream: TcpStream, response: MockResponse) {
        match response {
            MockResponse::Json(body) => {
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
            }
            MockResponse::Status(code, body) => {
                let _ = write!(
                    stream,
                    "HTTP/1.1 {code} Mock Error\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
            }
            MockResponse::Sse(events) => {
                let body = sse_body(&events);
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
            }
            MockResponse::SseInterrupt(events) => {
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n"
                );
                for event in events {
                    let payload = format!("data: {event}\n\n");
                    let _ = write!(stream, "{:x}\r\n{}\r\n", payload.len(), payload);
                }
                let _ = stream.flush();
                let _ = stream.shutdown(std::net::Shutdown::Both);
                return;
            }
            MockResponse::SseThenHang(events) => {
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n"
                );
                for event in events {
                    let payload = format!("data: {event}\n\n");
                    let _ = write!(stream, "{:x}\r\n{}\r\n", payload.len(), payload);
                }
                let _ = stream.flush();
                std::thread::sleep(std::time::Duration::from_secs(5));
                return;
            }
        }
        let _ = stream.flush();
    }

    /// Minimal in-memory AppState for run_agent_loop tests. Settings live only in
    /// memory and usage records go to a unique temp dir, so user settings/providers
    /// are never touched.
    fn test_app_state() -> AppState {
        AppState {
            settings: std::sync::RwLock::new(Settings::default()),
            explain_images: Mutex::new(std::collections::HashMap::new()),
            current_explain_image_id: Mutex::new(None),
            lens_busy: AtomicBool::new(false),
            explain_stream_generation: std::sync::atomic::AtomicU64::new(0),
            chat_stream_generations: Mutex::new(std::collections::HashMap::new()),
            chat_active_replies: Mutex::new(std::collections::HashSet::new()),
            pending_chat_tool_approvals: Mutex::new(std::collections::HashMap::new()),
            pending_chat_user_prompts: Mutex::new(std::collections::HashMap::new()),
            pending_python_runs: Mutex::new(std::collections::HashMap::new()),
            chat_create_conversation_lock: Mutex::new(()),
            chat_tool_list_cache: Mutex::new(std::collections::HashMap::new()),
            pending_chat_external_sends: Mutex::new(Vec::new()),
            pending_selection: Mutex::new(None),
            lens_freeze_frame_image_id: Mutex::new(None),
            key_cooldowns: Mutex::new(std::collections::HashMap::new()),
            active_key_idx: Mutex::new(std::collections::HashMap::new()),
            usage_dir: std::env::temp_dir().join(format!(
                "kivio-agent-loop-test-usage-{}",
                uuid::Uuid::new_v4()
            )),
            http: reqwest::Client::new(),
            #[cfg(target_os = "macos")]
            macos_ocr: crate::macos_ocr::MacOcrClient::disabled(),
            rapidocr: crate::rapidocr::RapidOcrClient::disabled(),
        }
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
        }
    }

    fn test_run_config<'a>(
        state: &'a AppState,
        base_url: &str,
        stream_enabled: bool,
    ) -> AgentRunConfig<'a> {
        AgentRunConfig {
            entry: AgentRunEntry::Send,
            state,
            conversation_id: "conversation".to_string(),
            run_id: "run".to_string(),
            message_id: "message".to_string(),
            generation: 1,
            provider: test_provider(base_url),
            model: "test-model".to_string(),
            runtime_messages: vec![
                serde_json::json!({ "role": "system", "content": "system prompt" }),
                serde_json::json!({ "role": "user", "content": "请读取文件" }),
            ],
            tools: vec![native_read_file_tool()],
            blocked_tool_calls: Vec::new(),
            settings: Settings::default(),
            effective_chat_tools: ChatToolsConfig {
                max_tool_rounds: Some(1),
                ..ChatToolsConfig::default()
            },
            language: "zh-CN".to_string(),
            has_image: false,
            thinking_enabled: false,
            stream_enabled,
            max_output_tokens: 1024,
            retry_attempts: 1,
            skill_registry: skills::SkillRegistry::default(),
            active_skill_id: None,
            active_skill_detail: None,
            assistant_snapshot: None,
            custom_system_prompt: String::new(),
            provider_tools_fallback_system_prompt: String::new(),
        }
    }

    /// Streaming planning step: one `read_file` tool call, then `[DONE]`.
    fn planning_tool_call_sse_events() -> Vec<String> {
        vec![
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_read","function":{"name":"read_file","arguments":"{\"path\":\"/tmp/kivio-test.txt\"}"}}]}}]}"#
                .to_string(),
            r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#.to_string(),
            "[DONE]".to_string(),
        ]
    }

    /// Non-stream planning step: one `read_file` tool call.
    fn planning_tool_call_json() -> String {
        serde_json::json!({
            "choices": [{
                "finish_reason": "tool_calls",
                "message": {
                    "role": "assistant",
                    "content": Value::Null,
                    "tool_calls": [{
                        "id": "call_read",
                        "type": "function",
                        "function": {
                            "name": "read_file",
                            "arguments": "{\"path\":\"/tmp/kivio-test.txt\"}"
                        }
                    }]
                }
            }]
        })
        .to_string()
    }

    fn test_round_context() -> ToolRoundContext<'static> {
        ToolRoundContext {
            conversation_id: "conversation",
            run_id: "run",
            message_id: "message",
            generation: 1,
            round: 1,
        }
    }

    fn pending_tool_call(id: &str, function_name: &str) -> PendingToolCall {
        let arguments = test_tool_arguments(function_name);
        PendingToolCall {
            id: id.to_string(),
            function_name: function_name.to_string(),
            arguments_raw: serde_json::to_string(&arguments).expect("serialize test arguments"),
            arguments,
            arguments_parse_error: None,
        }
    }

    fn test_tool_arguments(function_name: &str) -> Value {
        match function_name {
            "read_file" => serde_json::json!({ "path": "/tmp/kivio-test.txt" }),
            "web_fetch" => serde_json::json!({ "url": "https://example.com" }),
            "run_python" => serde_json::json!({ "code": "print(1)" }),
            "ask_user" => serde_json::json!({
                "questions": [
                    {
                        "id": "scope",
                        "prompt": "Which scope should I use?",
                        "options": [
                            { "id": "mvp", "label": "MVP" },
                            { "id": "full", "label": "Full" }
                        ]
                    }
                ]
            }),
            _ => serde_json::json!({}),
        }
    }

    #[test]
    fn visible_tool_segment_calls_skip_hidden_disabled_builtin_feedback() {
        let tools = vec![native_read_file_tool()];
        let blocked = vec![native_run_python_tool()];
        let calls = vec![
            pending_tool_call("call_read", "read_file"),
            pending_tool_call("call_blocked", "run_python"),
            pending_tool_call("call_hidden_disabled", "web_search"),
            pending_tool_call("call_unknown", "mcp__server__tool"),
        ];

        let visible = visible_tool_segment_calls(&tools, &blocked, &calls);

        assert_eq!(
            visible
                .iter()
                .map(|call| call.id.as_str())
                .collect::<Vec<_>>(),
            vec!["call_read", "call_blocked", "call_unknown"]
        );
    }

    #[test]
    fn reasoning_segment_order_precedes_text_in_same_step() {
        let mut builder = SegmentBuilder::new();
        let reasoning = builder.reserve(
            ChatMessageSegmentKind::Reasoning,
            ChatMessageSegmentPhase::ToolLoop,
            Some(1),
            Some(1),
            "step_1_reasoning",
        );
        let text = builder.reserve(
            ChatMessageSegmentKind::Text,
            ChatMessageSegmentPhase::ToolLoop,
            Some(1),
            Some(1),
            "step_1_text",
        );

        assert!(reasoning.order < text.order);
    }

    fn test_mcp_tool(name: &str, annotations: Value) -> ChatToolDefinition {
        ChatToolDefinition {
            id: format!("mcp__demo__{name}"),
            name: name.to_string(),
            description: "MCP test tool".to_string(),
            source: "mcp".to_string(),
            server_id: Some("demo".to_string()),
            server_name: Some("Demo".to_string()),
            input_schema: serde_json::json!({ "type": "object", "properties": {} }),
            sensitive: false,
            annotations: Some(annotations),
            output_schema: None,
        }
    }

    fn tool_call_ids(messages: &[Value]) -> Vec<&str> {
        messages
            .iter()
            .filter_map(|message| message.get("tool_call_id").and_then(Value::as_str))
            .collect()
    }

    #[test]
    fn tool_round_limit_reached_only_for_finite_limits_at_boundary() {
        assert!(!tool_round_limit_reached(None, 10));
        assert!(!tool_round_limit_reached(Some(3), 2));
        assert!(tool_round_limit_reached(Some(3), 3));
        assert!(tool_round_limit_reached(Some(3), 4));
    }

    #[tokio::test]
    async fn tool_round_runs_parallel_eligible_tools_concurrently_and_keeps_result_order() {
        let host = TestHost::default();
        let executor = RecordingExecutor::default();
        let settings = Settings::default();
        let tools = vec![native_read_file_tool(), native_web_fetch_tool()];
        let mut skill_cache = skills::SkillRunCache::default();

        let result = execute_tool_round(
            &host,
            &executor,
            &settings,
            test_round_context(),
            &tools,
            &[],
            vec![
                pending_tool_call("call_read", "read_file"),
                pending_tool_call("call_fetch", "web_fetch"),
            ],
            &mut skill_cache,
        )
        .await;

        assert_eq!(executor.max_active(), 2);
        let events = executor.events();
        let first_finish = events
            .iter()
            .position(|event| event.starts_with("finish:"))
            .expect("finish event");
        assert_eq!(
            first_finish, 2,
            "both calls should start before either finishes"
        );
        assert_eq!(result.response_messages.len(), 2);
        assert_eq!(
            result.response_messages[0]
                .get("tool_call_id")
                .and_then(Value::as_str),
            Some("call_read")
        );
        assert_eq!(
            result.response_messages[1]
                .get("tool_call_id")
                .and_then(Value::as_str),
            Some("call_fetch")
        );
        assert_eq!(result.tool_records.len(), 2);
        assert!(result
            .tool_records
            .iter()
            .all(|record| matches!(record.status, ToolCallStatus::Success)));
    }

    #[test]
    fn write_tools_stay_outside_parallel_whitelist() {
        let mut settings = Settings::default();
        settings.chat_tools.approval_policy = "auto".to_string();
        for tool in [
            crate::mcp::types::native_write_file_tool(),
            crate::mcp::types::native_edit_file_tool(),
        ] {
            assert!(
                !tool_call_parallel_eligible(&settings, &tool),
                "{} must stay serial even when approval is auto",
                tool.name
            );
        }
        assert!(
            tool_call_parallel_eligible(&settings, &native_read_file_tool()),
            "read-only tools remain parallel-eligible"
        );
    }

    #[tokio::test]
    async fn tool_round_runs_read_only_mcp_tools_concurrently() {
        let host = TestHost::default();
        let executor = RecordingExecutor::default();
        let settings = Settings::default();
        let tools = vec![
            test_mcp_tool("search_a", serde_json::json!({ "readOnlyHint": true })),
            test_mcp_tool("search_b", serde_json::json!({ "readOnlyHint": true })),
        ];
        let mut skill_cache = skills::SkillRunCache::default();

        let result = execute_tool_round(
            &host,
            &executor,
            &settings,
            test_round_context(),
            &tools,
            &[],
            vec![
                pending_tool_call("call_mcp_a", "search_a"),
                pending_tool_call("call_mcp_b", "search_b"),
            ],
            &mut skill_cache,
        )
        .await;

        assert_eq!(executor.max_active(), 2);
        assert_eq!(
            tool_call_ids(&result.response_messages),
            vec!["call_mcp_a", "call_mcp_b"]
        );
        assert!(result
            .tool_records
            .iter()
            .all(|record| matches!(record.status, ToolCallStatus::Success)));
    }

    #[tokio::test]
    async fn tool_round_keeps_ask_user_serial_between_parallel_safe_tools() {
        let host = TestHost::default();
        let executor = RecordingExecutor::default();
        let settings = Settings::default();
        let mut tools = vec![native_read_file_tool(), native_web_fetch_tool()];
        crate::chat::ask_user::append_tool_definitions(&mut tools);
        let mut skill_cache = skills::SkillRunCache::default();

        let result = execute_tool_round(
            &host,
            &executor,
            &settings,
            test_round_context(),
            &tools,
            &[],
            vec![
                pending_tool_call("call_read", "read_file"),
                pending_tool_call("call_ask", "ask_user"),
                pending_tool_call("call_fetch", "web_fetch"),
            ],
            &mut skill_cache,
        )
        .await;

        assert_eq!(executor.max_active(), 1);
        assert_eq!(
            executor.events(),
            vec![
                "start:read_file",
                "finish:read_file",
                "start:web_fetch",
                "finish:web_fetch"
            ]
        );
        assert_eq!(
            tool_call_ids(&result.response_messages),
            vec!["call_read", "call_ask", "call_fetch"]
        );
        let ask_user_record = result
            .tool_records
            .iter()
            .find(|record| record.id == "call_ask")
            .expect("ask_user record");
        assert!(matches!(ask_user_record.status, ToolCallStatus::Skipped));
        assert_eq!(
            ask_user_record.name,
            crate::chat::ask_user::ASK_USER_TOOL_NAME
        );
        assert_eq!(ask_user_record.trace_id.as_deref(), Some("run"));
        assert_eq!(
            ask_user_record.span_id.as_deref(),
            Some("tool_round_1_call_ask")
        );
    }

    #[tokio::test]
    async fn tool_round_keeps_destructive_mcp_tools_serial() {
        let host = TestHost::default();
        let executor = RecordingExecutor::default();
        let mut settings = Settings::default();
        settings.chat_tools.approval_policy = "auto".to_string();
        let tools = vec![test_mcp_tool(
            "write_remote",
            serde_json::json!({ "destructiveHint": true }),
        )];
        let mut skill_cache = skills::SkillRunCache::default();

        let result = execute_tool_round(
            &host,
            &executor,
            &settings,
            test_round_context(),
            &tools,
            &[],
            vec![
                pending_tool_call("call_mcp_write_1", "write_remote"),
                pending_tool_call("call_mcp_write_2", "write_remote"),
            ],
            &mut skill_cache,
        )
        .await;

        assert_eq!(executor.max_active(), 1);
        assert_eq!(
            tool_call_ids(&result.response_messages),
            vec!["call_mcp_write_1", "call_mcp_write_2"]
        );
    }

    #[tokio::test]
    async fn tool_round_keeps_open_world_mcp_tools_serial_even_when_read_only() {
        let host = TestHost::default();
        let executor = RecordingExecutor::default();
        let mut settings = Settings::default();
        settings.chat_tools.approval_policy = "auto".to_string();
        let tools = vec![test_mcp_tool(
            "remote_search",
            serde_json::json!({ "readOnlyHint": true, "openWorldHint": true }),
        )];
        let mut skill_cache = skills::SkillRunCache::default();

        let result = execute_tool_round(
            &host,
            &executor,
            &settings,
            test_round_context(),
            &tools,
            &[],
            vec![
                pending_tool_call("call_mcp_remote_1", "remote_search"),
                pending_tool_call("call_mcp_remote_2", "remote_search"),
            ],
            &mut skill_cache,
        )
        .await;

        assert_eq!(executor.max_active(), 1);
        assert_eq!(
            tool_call_ids(&result.response_messages),
            vec!["call_mcp_remote_1", "call_mcp_remote_2"]
        );
    }

    #[tokio::test]
    async fn tool_round_preserves_unknown_and_invalid_call_order() {
        let host = TestHost::default();
        let executor = RecordingExecutor::default();
        let settings = Settings::default();
        let tools = vec![native_read_file_tool(), native_web_fetch_tool()];
        let mut skill_cache = skills::SkillRunCache::default();
        let mut invalid_fetch = pending_tool_call("call_bad_args", "web_fetch");
        invalid_fetch.arguments_parse_error = Some("expected compact object".to_string());

        let result = execute_tool_round(
            &host,
            &executor,
            &settings,
            test_round_context(),
            &tools,
            &[],
            vec![
                pending_tool_call("call_read", "read_file"),
                pending_tool_call("call_fetch", "web_fetch"),
                pending_tool_call("call_missing", "missing_tool"),
                pending_tool_call("call_read_after_unknown", "read_file"),
                invalid_fetch,
                pending_tool_call("call_final", "read_file"),
            ],
            &mut skill_cache,
        )
        .await;

        let error_records = result
            .tool_records
            .iter()
            .filter(|record| matches!(record.status, ToolCallStatus::Error))
            .collect::<Vec<_>>();

        assert_eq!(executor.max_active(), 2);
        assert_eq!(
            tool_call_ids(&result.response_messages),
            vec![
                "call_read",
                "call_fetch",
                "call_missing",
                "call_read_after_unknown",
                "call_bad_args",
                "call_final"
            ]
        );
        assert_eq!(
            result
                .tool_records
                .iter()
                .map(|record| record.id.as_str())
                .collect::<Vec<_>>(),
            vec![
                "call_read",
                "call_fetch",
                "call_missing",
                "call_read_after_unknown",
                "call_bad_args",
                "call_final"
            ]
        );
        assert_eq!(
            error_records
                .iter()
                .map(|record| record.id.as_str())
                .collect::<Vec<_>>(),
            vec!["call_missing", "call_bad_args"]
        );
        assert!(error_records
            .iter()
            .all(|record| record.trace_id.as_deref() == Some("run")));
        assert_eq!(
            error_records
                .iter()
                .map(|record| record.span_id.as_deref())
                .collect::<Vec<_>>(),
            vec![
                Some("tool_round_1_call_missing"),
                Some("tool_round_1_call_bad_args")
            ]
        );
        let start_events = executor
            .events()
            .into_iter()
            .filter(|event| event.starts_with("start:"))
            .collect::<Vec<_>>();
        assert_eq!(start_events.len(), 4, "only executable tools should run");
    }

    #[tokio::test]
    async fn tool_round_records_plan_blocked_tool_as_skipped() {
        let host = TestHost::default();
        let executor = RecordingExecutor::default();
        let settings = Settings::default();
        let tools = vec![native_read_file_tool()];
        let blocked = vec![native_run_python_tool()];
        let mut skill_cache = skills::SkillRunCache::default();

        let result = execute_tool_round(
            &host,
            &executor,
            &settings,
            test_round_context(),
            &tools,
            &blocked,
            vec![pending_tool_call("call_py", "run_python")],
            &mut skill_cache,
        )
        .await;

        assert_eq!(executor.max_active(), 0);
        assert_eq!(result.response_messages.len(), 1);
        assert!(result.response_messages[0]
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .contains("blocked in Plan mode"));
        assert_eq!(result.tool_records.len(), 1);
        let record = &result.tool_records[0];
        assert_eq!(record.id, "call_py");
        assert_eq!(record.name, "run_python");
        assert!(matches!(record.status, ToolCallStatus::Skipped));
        assert!(record
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("blocked in Plan mode"));
        assert_eq!(record.trace_id.as_deref(), Some("run"));
        assert_eq!(record.span_id.as_deref(), Some("tool_round_1_call_py"));
    }

    #[tokio::test]
    async fn tool_round_cancels_unstarted_calls_after_running_tool_is_cancelled() {
        let host = TestHost::cancelling_after(Duration::from_millis(5));
        let executor = RecordingExecutor::default();
        let settings = Settings::default();
        let tools = vec![native_read_file_tool(), native_run_python_tool()];
        let mut skill_cache = skills::SkillRunCache::default();

        let result = execute_tool_round(
            &host,
            &executor,
            &settings,
            test_round_context(),
            &tools,
            &[],
            vec![
                pending_tool_call("call_read", "read_file"),
                pending_tool_call("call_py", "run_python"),
            ],
            &mut skill_cache,
        )
        .await;

        assert!(result.cancelled);
        assert_eq!(
            tool_call_ids(&result.response_messages),
            vec!["call_read", "call_py"]
        );
        assert_eq!(
            result
                .tool_records
                .iter()
                .map(|record| record.id.as_str())
                .collect::<Vec<_>>(),
            vec!["call_read", "call_py"]
        );
        assert!(result
            .tool_records
            .iter()
            .all(|record| matches!(record.status, ToolCallStatus::Cancelled)));
        let start_events = executor
            .events()
            .into_iter()
            .filter(|event| event.starts_with("start:"))
            .collect::<Vec<_>>();
        assert_eq!(
            start_events,
            vec!["start:read_file"],
            "remaining serial tools must not start after cancellation"
        );
    }

    #[test]
    fn cancelled_tool_round_result_preserves_replay_messages_for_storage() {
        let tool_record = ToolCallRecord {
            id: "call_read".to_string(),
            name: "read_file".to_string(),
            source: "native".to_string(),
            server_id: None,
            arguments: "{}".to_string(),
            status: ToolCallStatus::Cancelled,
            result_preview: None,
            error: Some("Tool call cancelled".to_string()),
            duration_ms: Some(5),
            started_at: Some(10),
            completed_at: Some(11),
            round: 1,
            sensitive: false,
            artifacts: Vec::new(),
            trace_id: Some("run".to_string()),
            span_id: Some("tool_round_1_call_read".to_string()),
            structured_content: None,
        };
        let assistant_message = serde_json::json!({
            "role": "assistant",
            "content": Value::Null,
            "tool_calls": [{
                "id": "call_read",
                "type": "function",
                "function": {
                    "name": "read_file",
                    "arguments": "{}",
                }
            }],
        });
        let tool_response = tool_message("call_read".to_string(), "Tool call cancelled");
        let result = cancelled_tool_round_run_result(
            "zh-CN",
            &["planning".to_string()],
            vec![tool_record.clone()],
            vec![ChatMessageSegment {
                id: "seg_1000_tool_call_read".to_string(),
                kind: ChatMessageSegmentKind::Tool,
                phase: ChatMessageSegmentPhase::ToolLoop,
                order: 1000,
                step_number: Some(1),
                round: Some(1),
                text: None,
                tool_call_id: Some("call_read".to_string()),
            }],
            vec![assistant_message.clone(), tool_response.clone()],
            vec![AgentStepResult {
                step_number: 1,
                phase: AgentPhase::ToolLoop,
                response_messages: vec![assistant_message.clone(), tool_response.clone()],
                tool_records: vec![tool_record],
                segments: Vec::new(),
                streamed: true,
                stop_reason: Some(AgentStopReason::Cancelled),
            }],
        );

        assert_eq!(result.content, "已停止生成。");
        assert_eq!(result.reasoning.as_deref(), Some("planning"));
        assert_eq!(result.tool_records.len(), 1);
        assert!(result.segments.iter().any(|segment| {
            segment.kind == ChatMessageSegmentKind::Tool
                && segment.tool_call_id.as_deref() == Some("call_read")
        }));
        assert!(result.segments.iter().any(|segment| {
            segment.kind == ChatMessageSegmentKind::Text
                && segment.phase == ChatMessageSegmentPhase::Synthesis
                && segment.text.as_deref() == Some("已停止生成。")
        }));
        assert!(matches!(
            result.tool_records[0].status,
            ToolCallStatus::Cancelled
        ));
        assert_eq!(result.api_messages.len(), 3);
        assert_eq!(
            result.api_messages[0]
                .get("tool_calls")
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(1)
        );
        assert_eq!(
            result.api_messages[1]
                .get("tool_call_id")
                .and_then(Value::as_str),
            Some("call_read")
        );
        assert_eq!(
            result.api_messages[2]
                .get("content")
                .and_then(Value::as_str),
            Some("已停止生成。")
        );
        assert_eq!(
            result.steps[0].stop_reason,
            Some(AgentStopReason::Cancelled)
        );
    }

    #[tokio::test]
    async fn tool_round_keeps_serial_only_tools_non_overlapping() {
        let host = TestHost::default();
        let executor = RecordingExecutor::default();
        let settings = Settings::default();
        let tools = vec![native_run_python_tool()];
        let mut skill_cache = skills::SkillRunCache::default();

        let result = execute_tool_round(
            &host,
            &executor,
            &settings,
            test_round_context(),
            &tools,
            &[],
            vec![
                pending_tool_call("call_py_1", "run_python"),
                pending_tool_call("call_py_2", "run_python"),
            ],
            &mut skill_cache,
        )
        .await;

        assert_eq!(executor.max_active(), 1);
        assert_eq!(
            executor.events(),
            vec![
                "start:run_python",
                "finish:run_python",
                "start:run_python",
                "finish:run_python"
            ]
        );
        assert_eq!(result.response_messages.len(), 2);
        assert_eq!(
            result.response_messages[0]
                .get("tool_call_id")
                .and_then(Value::as_str),
            Some("call_py_1")
        );
        assert_eq!(
            result.response_messages[1]
                .get("tool_call_id")
                .and_then(Value::as_str),
            Some("call_py_2")
        );
    }

    // ===== Fallback scenarios (6 synthesis/planning fallbacks in run_agent_loop) =====

    #[test]
    fn fallback_responses_are_bilingual() {
        assert_eq!(
            empty_synthesis_fallback_response("zh-CN"),
            "工具调用已经完成，但模型没有返回最终总结。上方工具结果已保存在本轮回复中，你可以继续追问，或让我重新生成总结。"
        );
        assert_eq!(
            empty_synthesis_fallback_response("en-US"),
            "The tool calls completed, but the model did not return a final summary. The tool results above were saved with this reply; you can continue from them or regenerate the summary."
        );
        assert_eq!(
            synthesis_failed_fallback_response("zh-CN"),
            "工具调用已经完成，但最终总结生成失败。上方工具结果已保存在本轮回复中，你可以继续追问，或让我重新生成总结。"
        );
        assert_eq!(
            synthesis_failed_fallback_response("en-US"),
            "The tool calls completed, but final summary generation failed. The tool results above were saved with this reply; you can continue from them or regenerate the summary."
        );
        assert_eq!(
            tool_planning_failed_fallback_response("zh-CN"),
            "工具调用参数生成失败，这一步还没有真正执行写入。主对话已保留，你可以让我缩小范围、改用补丁，或重新生成。"
        );
        assert_eq!(
            tool_planning_failed_fallback_response("en-US"),
            "Tool-call argument generation failed before the write actually ran. This conversation was preserved; you can ask me to narrow the scope, use a patch, or regenerate."
        );
        assert_eq!(stopped_generation_content("zh-CN"), "已停止生成。");
        assert_eq!(stopped_generation_content("en-US"), "Generation stopped.");
    }

    /// Fallback A (helper level, deterministic): a planning stream dies while tool
    /// argument drafts are in flight; the run result must mark every draft record
    /// as error, emit the bilingual fallback, and finish with stream_outcome "error".
    #[test]
    fn tool_planning_failed_run_result_marks_drafts_error_and_emits_fallback() {
        let state = test_app_state();
        let config = test_run_config(&state, "http://127.0.0.1:9/v1", true);
        let host = TestHost::default();

        let mut segment_builder = SegmentBuilder::new();
        let _reasoning_segment = segment_builder.reserve(
            ChatMessageSegmentKind::Reasoning,
            ChatMessageSegmentPhase::ToolLoop,
            Some(1),
            Some(1),
            "step_1_reasoning",
        );
        let planning_text_segment = segment_builder.reserve(
            ChatMessageSegmentKind::Text,
            ChatMessageSegmentPhase::ToolLoop,
            Some(1),
            Some(1),
            "step_1_text",
        );
        let tracker = ToolCallDraftTracker::new(
            vec![native_write_file_tool()],
            1,
            Some(1),
            segment_builder.next_order(),
        );
        let mut sink = AgentStreamSink::new(
            &host,
            "conversation",
            "run",
            "message",
            true,
            None,
            None,
            Some(tracker.clone()),
        );
        sink.emit(StreamPart::ToolCallStart {
            id: "call_write".to_string(),
            name: "write_file".to_string(),
        })
        .expect("tool call start should emit");
        sink.emit(StreamPart::ToolCallDelta {
            id: "call_write".to_string(),
            delta: "{\"path\":\"large.html\",\"content\":\"".to_string(),
        })
        .expect("tool call delta should emit");
        assert!(tracker.has_started());

        let result = tool_planning_failed_run_result(
            &host,
            &config,
            segment_builder,
            planning_text_segment,
            tracker,
            &["planning thought".to_string()],
            Vec::new(),
            Vec::new(),
            "Chat tools planning read body failed".to_string(),
        );

        let fallback = tool_planning_failed_fallback_response("zh-CN");
        assert_eq!(result.stream_outcome, "error");
        assert_eq!(result.content, fallback);
        assert_eq!(result.reasoning.as_deref(), Some("planning thought"));
        assert_eq!(result.tool_records.len(), 1);
        assert_eq!(result.tool_records[0].id, "call_write");
        assert!(matches!(
            result.tool_records[0].status,
            ToolCallStatus::Error
        ));
        assert_eq!(
            result.tool_records[0].error.as_deref(),
            Some("Chat tools planning read body failed")
        );
        assert!(result.tool_records[0].completed_at.is_some());

        // The error record was re-emitted through the host (after the pending draft).
        let emitted = host.recorded_tool_records();
        let last_emitted = emitted.last().expect("error record emitted");
        assert_eq!(last_emitted.id, "call_write");
        assert!(matches!(last_emitted.status, ToolCallStatus::Error));

        // Draft tool segment is preserved and the fallback text segment is
        // synthesis-phase with no round.
        assert!(result.segments.iter().any(|segment| {
            segment.kind == ChatMessageSegmentKind::Tool
                && segment.tool_call_id.as_deref() == Some("call_write")
        }));
        let fallback_segment = result
            .segments
            .iter()
            .find(|segment| segment.kind == ChatMessageSegmentKind::Text)
            .expect("fallback text segment");
        assert_eq!(fallback_segment.phase, ChatMessageSegmentPhase::Synthesis);
        assert_eq!(fallback_segment.round, None);
        assert_eq!(fallback_segment.text.as_deref(), Some(fallback.as_str()));

        // Fallback delta + a single "done" done event.
        assert!(host
            .recorded_deltas()
            .iter()
            .any(|delta| delta.delta == fallback));
        assert_eq!(
            host.recorded_dones(),
            vec![("done".to_string(), fallback.clone())]
        );

        // The final assistant message is pushed unconditionally here.
        assert_eq!(result.api_messages.len(), 1);
        assert_eq!(
            result.api_messages[0]
                .get("content")
                .and_then(Value::as_str),
            Some(fallback.as_str())
        );

        let last_step = result.steps.last().expect("fallback step");
        assert_eq!(last_step.stop_reason, Some(AgentStopReason::Natural));
        assert_eq!(last_step.tool_records.len(), 1);
    }

    /// Fallback A (integration): the provider stream breaks mid-connection after a
    /// tool-call draft has started; run_agent_loop must return Ok with
    /// stream_outcome "error" instead of bubbling an invoke error.
    #[tokio::test]
    async fn run_loop_stream_planning_interrupt_after_tool_draft_returns_error_result() {
        let server = MockModelServer::start(vec![MockResponse::SseInterrupt(vec![
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_read","function":{"name":"read_file","arguments":"{\"path\":\"/tmp/"}}]}}]}"#
                .to_string(),
        ])]);
        let state = test_app_state();
        let config = test_run_config(&state, &server.base_url, true);
        let host = TestHost::default();
        let executor = RecordingExecutor::default();

        let result = run_agent_loop(config, &host, &executor)
            .await
            .expect("planning interruption with tool draft must not bubble Err");

        let fallback = tool_planning_failed_fallback_response("zh-CN");
        assert_eq!(result.stream_outcome, "error");
        assert_eq!(result.content, fallback);
        assert_eq!(result.tool_records.len(), 1);
        assert_eq!(result.tool_records[0].id, "call_read");
        assert!(matches!(
            result.tool_records[0].status,
            ToolCallStatus::Error
        ));
        assert!(
            executor.events().is_empty(),
            "interrupted tool drafts must never execute"
        );
        assert_eq!(
            host.recorded_dones(),
            vec![("done".to_string(), fallback.clone())]
        );
        assert_eq!(result.api_messages.len(), 1);
        assert_eq!(
            result.api_messages[0]
                .get("content")
                .and_then(Value::as_str),
            Some(fallback.as_str())
        );
    }

    /// Fallback B: streamed synthesis request fails (HTTP 400) after a successful
    /// tool round; the tool records must survive with the bilingual fallback text.
    #[tokio::test]
    async fn run_loop_stream_synthesis_failure_preserves_tool_records_with_fallback() {
        let server = MockModelServer::start(vec![
            MockResponse::Sse(planning_tool_call_sse_events()),
            MockResponse::Status(400, r#"{"error":"mock synthesis failure"}"#.to_string()),
        ]);
        let state = test_app_state();
        let config = test_run_config(&state, &server.base_url, true);
        let host = TestHost::default();
        let executor = RecordingExecutor::default();

        let result = run_agent_loop(config, &host, &executor)
            .await
            .expect("synthesis failure after tool records must not bubble Err");

        let fallback = synthesis_failed_fallback_response("zh-CN");
        assert_eq!(result.stream_outcome, "error");
        assert_eq!(result.content, fallback);
        assert_eq!(result.tool_records.len(), 1);
        assert_eq!(result.tool_records[0].id, "call_read");
        assert!(matches!(
            result.tool_records[0].status,
            ToolCallStatus::Success
        ));

        // assistant tool_calls message + tool result + final fallback message.
        assert_eq!(result.api_messages.len(), 3);
        assert_eq!(
            result.api_messages[2]
                .get("content")
                .and_then(Value::as_str),
            Some(fallback.as_str())
        );

        // Planning never emits done while tool calls are pending; the only done is
        // the fallback "done".
        assert_eq!(
            host.recorded_dones(),
            vec![("done".to_string(), fallback.clone())]
        );
        let fallback_delta = host
            .recorded_deltas()
            .into_iter()
            .find(|delta| delta.delta == fallback)
            .expect("fallback delta emitted");
        assert_eq!(fallback_delta.reasoning_delta, None);
        let segment = fallback_delta.segment.expect("fallback delta has segment");
        assert_eq!(segment.kind, ChatMessageSegmentKind::Text);
        assert_eq!(segment.phase, ChatMessageSegmentPhase::Synthesis);

        assert_eq!(result.steps.len(), 1);
        assert_eq!(result.steps[0].stop_reason, Some(AgentStopReason::StepLimit));
    }

    /// Fallback C: streamed synthesis is cancelled after tool results exist; the run
    /// returns Ok("cancelled") with the stopped-generation placeholder content.
    #[tokio::test]
    async fn run_loop_stream_synthesis_cancelled_returns_cancelled_with_stopped_content() {
        let server = MockModelServer::start(vec![
            MockResponse::Sse(planning_tool_call_sse_events()),
            MockResponse::SseThenHang(Vec::new()),
        ]);
        let state = test_app_state();
        let config = test_run_config(&state, &server.base_url, true);
        let cancel_flag = Arc::new(AtomicBool::new(false));
        let host = TestHost::with_cancel_flag(cancel_flag.clone());
        let executor = CancelAfterToolExecutor { cancel_flag };

        let result = run_agent_loop(config, &host, &executor)
            .await
            .expect("cancelled synthesis with tool records must not bubble Err");

        assert_eq!(result.stream_outcome, "cancelled");
        assert_eq!(result.content, stopped_generation_content("zh-CN"));
        assert_eq!(result.tool_records.len(), 1);
        assert!(matches!(
            result.tool_records[0].status,
            ToolCallStatus::Success
        ));

        let dones = host.recorded_dones();
        assert_eq!(dones.len(), 1, "exactly one done event");
        assert_eq!(dones[0].0, "cancelled");

        assert_eq!(
            result
                .api_messages
                .last()
                .and_then(|message| message.get("content"))
                .and_then(Value::as_str),
            Some("已停止生成。")
        );
        assert!(result.segments.iter().any(|segment| {
            segment.kind == ChatMessageSegmentKind::Text
                && segment.text.as_deref() == Some("已停止生成。")
        }));
    }

    /// Fallback C variant: synthesis streamed some text before cancellation; the
    /// partial text must be kept instead of the stopped-generation placeholder.
    #[tokio::test]
    async fn run_loop_stream_synthesis_cancelled_keeps_partial_content() {
        let server = MockModelServer::start(vec![
            MockResponse::Sse(planning_tool_call_sse_events()),
            MockResponse::SseThenHang(vec![
                r#"{"choices":[{"delta":{"content":"部分回答"}}]}"#.to_string()
            ]),
        ]);
        let state = test_app_state();
        let config = test_run_config(&state, &server.base_url, true);
        let host = TestHost::cancelling_on_first_text_delta();
        let executor = RecordingExecutor::default();

        let result = run_agent_loop(config, &host, &executor)
            .await
            .expect("cancelled synthesis with partial content must not bubble Err");

        assert_eq!(result.stream_outcome, "cancelled");
        assert_eq!(result.content, "部分回答");
        assert_eq!(result.tool_records.len(), 1);
        let dones = host.recorded_dones();
        assert_eq!(dones.len(), 1);
        assert_eq!(dones[0], ("cancelled".to_string(), "部分回答".to_string()));
        assert_eq!(
            result
                .api_messages
                .last()
                .and_then(|message| message.get("content"))
                .and_then(Value::as_str),
            Some("部分回答")
        );
    }

    /// Fallback D: streamed synthesis returns an empty answer after tool results;
    /// the loop substitutes the bilingual fallback and completes normally
    /// (stream_outcome "completed", not "error").
    #[tokio::test]
    async fn run_loop_stream_synthesis_empty_output_uses_fallback_and_completes() {
        let server = MockModelServer::start(vec![
            MockResponse::Sse(planning_tool_call_sse_events()),
            MockResponse::Sse(vec!["[DONE]".to_string()]),
        ]);
        let state = test_app_state();
        let config = test_run_config(&state, &server.base_url, true);
        let host = TestHost::default();
        let executor = RecordingExecutor::default();

        let result = run_agent_loop(config, &host, &executor)
            .await
            .expect("empty synthesis with tool records must not bubble Err");

        let fallback = empty_synthesis_fallback_response("zh-CN");
        assert_eq!(result.stream_outcome, "completed");
        assert_eq!(result.content, fallback);
        assert_eq!(result.tool_records.len(), 1);
        assert!(matches!(
            result.tool_records[0].status,
            ToolCallStatus::Success
        ));
        assert_eq!(
            host.recorded_dones(),
            vec![("done".to_string(), fallback.clone())]
        );

        assert_eq!(result.steps.len(), 2);
        assert_eq!(result.steps[0].stop_reason, Some(AgentStopReason::StepLimit));
        let final_step = &result.steps[1];
        assert_eq!(final_step.phase, AgentPhase::Synthesis);
        assert_eq!(final_step.stop_reason, Some(AgentStopReason::Natural));

        assert!(result.segments.iter().any(|segment| {
            segment.kind == ChatMessageSegmentKind::Text
                && segment.phase == ChatMessageSegmentPhase::Synthesis
                && segment.text.as_deref() == Some(fallback.as_str())
        }));
        assert_eq!(
            result
                .api_messages
                .last()
                .and_then(|message| message.get("content"))
                .and_then(Value::as_str),
            Some(fallback.as_str())
        );
    }

    /// Fallback E: non-streamed synthesis request fails (HTTP 400) after a
    /// successful tool round; tool records survive with the failure fallback.
    #[tokio::test]
    async fn run_loop_nonstream_synthesis_failure_preserves_tool_records_with_fallback() {
        let server = MockModelServer::start(vec![
            MockResponse::Json(planning_tool_call_json()),
            MockResponse::Status(400, r#"{"error":"mock synthesis failure"}"#.to_string()),
        ]);
        let state = test_app_state();
        let config = test_run_config(&state, &server.base_url, false);
        let host = TestHost::default();
        let executor = RecordingExecutor::default();

        let result = run_agent_loop(config, &host, &executor)
            .await
            .expect("non-stream synthesis failure after tool records must not bubble Err");

        let fallback = synthesis_failed_fallback_response("zh-CN");
        assert_eq!(result.stream_outcome, "error");
        assert_eq!(result.content, fallback);
        assert_eq!(result.tool_records.len(), 1);
        assert!(matches!(
            result.tool_records[0].status,
            ToolCallStatus::Success
        ));
        assert!(host
            .recorded_deltas()
            .iter()
            .any(|delta| delta.delta == fallback));
        assert_eq!(
            host.recorded_dones(),
            vec![("done".to_string(), fallback.clone())]
        );
        assert_eq!(
            result
                .api_messages
                .last()
                .and_then(|message| message.get("content"))
                .and_then(Value::as_str),
            Some(fallback.as_str())
        );
    }

    /// Fallback F: non-streamed synthesis returns empty content after tool results;
    /// the bilingual fallback replaces it and reasoning is still passed through.
    #[tokio::test]
    async fn run_loop_nonstream_synthesis_empty_output_uses_fallback() {
        let empty_synthesis = serde_json::json!({
            "choices": [{
                "finish_reason": "stop",
                "message": {
                    "role": "assistant",
                    "content": "",
                    "reasoning_content": "synthesis reasoning"
                }
            }]
        })
        .to_string();
        let server = MockModelServer::start(vec![
            MockResponse::Json(planning_tool_call_json()),
            MockResponse::Json(empty_synthesis),
        ]);
        let state = test_app_state();
        let config = test_run_config(&state, &server.base_url, false);
        let host = TestHost::default();
        let executor = RecordingExecutor::default();

        let result = run_agent_loop(config, &host, &executor)
            .await
            .expect("non-stream empty synthesis with tool records must not bubble Err");

        let fallback = empty_synthesis_fallback_response("zh-CN");
        assert_eq!(result.stream_outcome, "completed");
        assert_eq!(result.content, fallback);
        assert_eq!(result.reasoning.as_deref(), Some("synthesis reasoning"));
        assert_eq!(result.tool_records.len(), 1);
        assert_eq!(
            host.recorded_dones(),
            vec![("done".to_string(), fallback.clone())]
        );

        let final_message = result.api_messages.last().expect("final api message");
        assert_eq!(
            final_message.get("content").and_then(Value::as_str),
            Some(fallback.as_str())
        );
        assert_eq!(
            final_message
                .get("reasoning_content")
                .and_then(Value::as_str),
            Some("synthesis reasoning")
        );

        let final_step = result.steps.last().expect("final step");
        assert_eq!(final_step.phase, AgentPhase::Synthesis);
        assert_eq!(final_step.stop_reason, Some(AgentStopReason::Natural));
    }
}
