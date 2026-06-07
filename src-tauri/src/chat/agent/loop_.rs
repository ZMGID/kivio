use serde_json::Value;

use crate::chat::model::{
    generate_request_from_openai_messages, AnthropicMessagesProvider, AppleLocalProvider,
    GenerateOptions, GenerateOutput, LanguageModelProvider, ModelError, OpenAiChatProvider,
    PendingToolCall,
};
use crate::chat::types::{ToolCallRecord, ToolCallStatus};
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
    assistant_api_message_for_tool_calls, empty_assistant_response_error,
    extract_reasoning_content, extract_tool_calls, final_assistant_api_message,
    final_response_from_planning_message, is_tools_unsupported_error, merge_reasoning,
    patch_system_message, sanitize_assistant_text_response,
};
use super::stream::{should_emit_done, validate_stream_output, AgentStreamSink, ChatStreamOutput};
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

pub async fn run_agent_loop(
    mut config: AgentRunConfig<'_>,
    host: &dyn AgentHost,
    executor: &dyn ToolExecutor,
) -> Result<AgentRunResult, String> {
    let mut runtime_messages = std::mem::take(&mut config.runtime_messages);
    let mut tools = std::mem::take(&mut config.tools);
    let mut generated_api_messages = Vec::new();
    let mut tool_records = Vec::new();
    let mut planning_reasoning_parts: Vec<String> = Vec::new();
    let mut provider_tools_unsupported = false;
    let mut planning_final_message: Option<Value> = None;
    let mut planning_final_already_streamed = false;
    let mut steps = Vec::new();
    let mut step_number = 0u8;

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
                        streamed: false,
                        stop_reason: Some(AgentStopReason::ProviderToolsUnsupported),
                    });
                    break;
                }
                Err(err) => return Err(err),
            };
            let tool_calls = extract_tool_calls(&message);
            if tool_calls.is_empty() {
                planning_final_message = Some(message.clone());
                steps.push(AgentStepResult {
                    step_number,
                    phase: AgentPhase::ToolLoop,
                    response_messages: vec![message],
                    tool_records: Vec::new(),
                    streamed: planning_final_already_streamed,
                    stop_reason: Some(AgentStopReason::Natural),
                });
                break;
            }
            planning_final_already_streamed = false;
            if let Some(reasoning) = extract_reasoning_content(&message) {
                if !config.stream_enabled {
                    host.emit_stream_delta(
                        &config.conversation_id,
                        &config.run_id,
                        &config.message_id,
                        "",
                        Some(&reasoning),
                    );
                }
                planning_reasoning_parts.push(reasoning);
            }

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
                    generated_api_messages,
                    steps,
                ));
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
            api_messages: generated_api_messages,
            steps,
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

    let (response, reasoning) = if config.stream_enabled {
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
        )
        .await
        .map_err(|err| err.to_string())?;
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
                    content: stored_content,
                    reasoning,
                    tool_records,
                    api_messages: generated_api_messages,
                    steps,
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
                (fallback, reasoning)
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
            (response, reasoning)
        }
    } else {
        let message = tokio::select! {
            result = call_chat_completion_message(
                config.state,
                &config.provider,
                &config.model,
                runtime_messages,
                None,
                config.retry_attempts,
                config.thinking_enabled,
                config.max_output_tokens,
                "Chat API",
            ) => result?,
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
            (fallback, reasoning)
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
            (response, reasoning)
        }
    };

    steps.push(AgentStepResult {
        step_number,
        phase,
        response_messages: Vec::new(),
        tool_records: Vec::new(),
        streamed: config.stream_enabled,
        stop_reason: Some(AgentStopReason::Natural),
    });

    Ok(AgentRunResult {
        content: response,
        reasoning,
        tool_records,
        api_messages: generated_api_messages,
        steps,
    })
}

async fn execute_tool_round(
    host: &dyn AgentHost,
    executor: &dyn ToolExecutor,
    settings: &Settings,
    ctx: ToolRoundContext<'_>,
    tools: &[ChatToolDefinition],
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
            let result = unknown_or_disabled_tool_result(host, &ctx, tool_call);
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
    tool_call: PendingToolCall,
) -> ToolExecutionResult {
    let disabled = disabled_tool_content(&tool_call);
    let record = if disabled.is_none() {
        let error = format!("Unknown tool requested: {}", tool_call.function_name);
        let record = unknown_tool_record(&tool_call, ctx.round, error);
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

fn invalid_tool_arguments_result(
    host: &dyn AgentHost,
    ctx: &ToolRoundContext<'_>,
    tool_call: &PendingToolCall,
    tool: &ChatToolDefinition,
    error: String,
) -> ToolExecutionResult {
    let record = invalid_tool_arguments_record(tool_call, tool, ctx.round, error);
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
    tool.source == "native"
        && matches!(tool.name.as_str(), "web_search" | "web_fetch" | "read_file")
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
    );
    let mut sink = AgentStreamSink::new(
        host,
        conversation_id,
        run_id,
        message_id,
        matches!(policy, AgentStreamPolicy::PlanningNoDoneUntilNoTools),
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
    let (snapshot_content, snapshot_reasoning) = sink.snapshot();
    let stream_output = ChatStreamOutput::from_generate_output_with_snapshot(
        output,
        snapshot_content,
        snapshot_reasoning,
    );
    validate_stream_output(label, policy, &stream_output).map_err(|err| {
        host.emit_stream_done(conversation_id, run_id, message_id, "error", "");
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

fn stopped_generation_content(language: &str) -> String {
    if language.starts_with("zh") {
        "已停止生成。".to_string()
    } else {
        "Generation stopped.".to_string()
    }
}

fn cancelled_tool_round_run_result(
    language: &str,
    planning_reasoning_parts: &[String],
    tool_records: Vec<ToolCallRecord>,
    mut generated_api_messages: Vec<Value>,
    steps: Vec<AgentStepResult>,
) -> AgentRunResult {
    let stopped_content = stopped_generation_content(language);
    if !generated_api_messages.is_empty() {
        generated_api_messages.push(final_assistant_api_message(&stopped_content, None));
    }
    AgentRunResult {
        content: stopped_content,
        reasoning: merge_reasoning(planning_reasoning_parts, None),
        tool_records,
        api_messages: generated_api_messages,
        steps,
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
        ProviderApiFormat::AppleLocal => {
            AppleLocalProvider::new(state.apple_intelligence.clone())
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
        ProviderApiFormat::AppleLocal => {
            AppleLocalProvider::new(state.apple_intelligence.clone())
                .stream(request, sink)
                .await
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    };

    use tokio::time::{sleep, Duration};

    use super::*;
    use crate::chat::types::ToolCallStatus;
    use crate::mcp::types::{
        native_read_file_tool, native_run_python_tool, native_web_fetch_tool, McpToolCallResult,
    };

    #[derive(Default)]
    struct TestHost {
        records: Mutex<Vec<ToolCallRecord>>,
        cancel_after: Option<Duration>,
    }

    impl TestHost {
        fn cancelling_after(delay: Duration) -> Self {
            Self {
                records: Mutex::new(Vec::new()),
                cancel_after: Some(delay),
            }
        }
    }

    impl AgentHost for TestHost {
        fn emit_stream_delta(
            &self,
            _conversation_id: &str,
            _run_id: &str,
            _message_id: &str,
            _delta: &str,
            _reasoning_delta: Option<&str>,
        ) {
        }

        fn emit_stream_done(
            &self,
            _conversation_id: &str,
            _run_id: &str,
            _message_id: &str,
            _reason: &str,
            _full: &str,
        ) {
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

        fn is_generation_active(&self, _conversation_id: &str, _generation: u64) -> bool {
            true
        }

        fn wait_for_generation_inactive<'a>(
            &'a self,
            _conversation_id: &'a str,
            _generation: u64,
        ) -> super::super::host::AgentHostFuture<'a, ()> {
            let cancel_after = self.cancel_after;
            Box::pin(async move {
                if let Some(delay) = cancel_after {
                    sleep(delay).await;
                } else {
                    std::future::pending::<()>().await
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
                })
            })
        }
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
        PendingToolCall {
            id: id.to_string(),
            function_name: function_name.to_string(),
            arguments: serde_json::json!({}),
            arguments_raw: "{}".to_string(),
            arguments_parse_error: None,
        }
    }

    fn tool_call_ids(messages: &[Value]) -> Vec<&str> {
        messages
            .iter()
            .filter_map(|message| message.get("tool_call_id").and_then(Value::as_str))
            .collect()
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
            result
                .tool_records
                .iter()
                .filter(|record| matches!(record.status, ToolCallStatus::Error))
                .map(|record| record.id.as_str())
                .collect::<Vec<_>>(),
            vec!["call_missing", "call_bad_args"]
        );
        let start_events = executor
            .events()
            .into_iter()
            .filter(|event| event.starts_with("start:"))
            .collect::<Vec<_>>();
        assert_eq!(start_events.len(), 4, "only executable tools should run");
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
            vec![assistant_message.clone(), tool_response.clone()],
            vec![AgentStepResult {
                step_number: 1,
                phase: AgentPhase::ToolLoop,
                response_messages: vec![assistant_message.clone(), tool_response.clone()],
                tool_records: vec![tool_record],
                streamed: true,
                stop_reason: Some(AgentStopReason::Cancelled),
            }],
        );

        assert_eq!(result.content, "已停止生成。");
        assert_eq!(result.reasoning.as_deref(), Some("planning"));
        assert_eq!(result.tool_records.len(), 1);
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
}
