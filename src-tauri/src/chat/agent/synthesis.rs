use super::content::{ContentKind, ContentBlock, AgentToolStatus};
use super::history::RuntimeMessage;
use crate::chat::model::{GenerateOutput, ModelError, ModelErrorKind, ModelMessage, ModelRole};

use super::finalize::{
    empty_synthesis_fallback_response, segment_phase_for_agent_phase,
    synthesis_failed_fallback_response, RunResultBuilder,
};
use super::loop_::{LoopEnv, RunState};
use super::recovery::{self, RecoveryAction};
use super::stop::{empty_assistant_response_error, merge_reasoning, sanitize_assistant_text_response};
use super::stream::WebSearchCardTracker;
use super::types::{AgentPhase, AgentRunConfig, AgentRunResult, AgentStreamPolicy};

pub(crate) struct SynthesisCompleted {
    pub(crate) replay: Option<crate::chat::model::ModelMessage>,
    pub(crate) response: String,
    pub(crate) reasoning: Option<String>,
    pub(crate) response_reasoning: Option<String>,
    pub(crate) response_segment: ContentBlock,
    pub(crate) response_reasoning_segment: ContentBlock,
}

pub(crate) enum SynthesisFlow {
    Completed(SynthesisCompleted),
    Early(AgentRunResult),
}

pub(crate) async fn synthesis_step(
    env: &LoopEnv<'_>,
    state: &mut RunState,
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
    let send_messages = super::compaction::maybe_compact_send_view(env, state, &[]).await?;
    if state.compaction_blocked {
        let content = recovery::assemble_results_from_tool_records(
            &state.tool_records,
            &config.language,
            recovery::FailureKind::ContextOverflow,
        );
        let content = if content.trim().is_empty() {
            recovery::overflow_static_message(&config.language)
        } else {
            content
        };
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
            config.provider.name.clone(),
        ))
    } else {
        None
    };

    let stream = recovery::stream_with_truncation_recovery(
        env,
        state,
        super::provider_runtime::StreamRequest {
            host,
            provider: &config.provider,
            model: &config.model,
            messages: send_messages,
            tools: None,
            tool_choice: crate::chat::model::ToolChoice::Auto,
            retry_attempts: config.retry_attempts,
            thinking_enabled: config.thinking_enabled,
            thinking_level: config.thinking_level.clone(),
            builtin_web_search: config.builtin_web_search_active(),
            max_output_tokens: config.max_output_tokens,
            conversation_id: &config.conversation_id,
            message_id: &config.message_id,
            label: "Chat stream",
            policy: synthesis_stream_policy,
            text_segment: Some(response_segment.clone()),
            reasoning_segment: Some(response_reasoning_segment.clone()),
            tool_draft_tracker: None,
            web_search_tracker: synth_web_search_tracker.clone(),
        },
    )
    .await;
    let mut stream = match stream {
        Ok(stream) => stream,
        Err(err) if !state.tool_records.is_empty() => {
            eprintln!("Chat synthesis stream failed after tool records; recovering: {err}");
            let recovered = recover_synthesis(env, state, &err).await?;
            let replay = recovered.replay;
            let content = if recovered.text.trim().is_empty() {
                synthesis_failed_fallback_response(&config.language)
            } else {
                recovered.text
            };
            return Ok(SynthesisFlow::Early(
                RunResultBuilder::new(host, content)
                    .segment(&response_segment)
                    .emit_done("done")
                    .outcome(super::types::AgentRunStatus::Recovered)
                    .replay(replay.clone())
                    .degraded(state.degraded.take())
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
        if !state.tool_records.is_empty() {
            return Ok(SynthesisFlow::Early(
                RunResultBuilder::new(host, stream.text.clone())
                    .segment(&response_segment)
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
        let partial = sanitize_assistant_text_response(&stream.text);
        if partial.trim().is_empty() {
            return Err("cancelled".to_string());
        }
        // Plain-text streaming was cancelled after partial output; the stream
        // layer already emitted the single done("cancelled") event. Preserve
        // the generated text instead of dropping the whole turn.
        return Ok(SynthesisFlow::Early(
            RunResultBuilder::new(host, partial)
                .segment(&response_segment)
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
    let final_reasoning_for_api = stream.reasoning.clone();
    let reasoning = merge_reasoning(&state.planning_reasoning_parts, stream.reasoning.clone());
    let response = sanitize_assistant_text_response(&stream.text);
    let replay;
    let (response, reasoning, response_reasoning) = if response.trim().is_empty() {
        if !state.tool_records.is_empty() {
            log_empty_synthesis_output(config, phase, &stream, state.tool_records.len());
            let recovered = recover_synthesis(
                env,
                state,
                &ModelError::with_kind("", ModelErrorKind::EmptyResponse),
            )
            .await?;
            replay = recovered.replay;
            let content = if recovered.text.trim().is_empty() {
                empty_synthesis_fallback_response(&config.language)
            } else {
                recovered.text
            };
            let fallback = RunResultBuilder::new(host, content)
                .emit_segment_opt(Some(response_segment.clone()))
                .api_reasoning(final_reasoning_for_api.clone())
                .emit_done("done")
                .replay(replay.clone())
                .emit_and_record(&mut state.generated_api_messages);
            (fallback, reasoning, final_reasoning_for_api)
        } else {
            return Err(empty_assistant_response_error("Chat stream"));
        }
    } else {
        let message = stream.replay_message();
        if !state.generated_api_messages.is_empty()
            || stream.canonical_message.is_some()
            || !stream.reasoning_items.is_empty()
        {
            state
                .generated_api_messages
                .push(RuntimeMessage::model(message.clone()));
        }
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

    Ok(SynthesisFlow::Completed(SynthesisCompleted {
        replay,
        response,
        reasoning,
        response_reasoning,
        response_segment,
        response_reasoning_segment,
    }))
}

fn last_user_text(messages: &[RuntimeMessage]) -> Option<String> {
    messages
        .iter()
        .rev()
        .filter(|message| message.role() == Some(ModelRole::User))
        .filter(|message| !crate::chat::todo::is_reminder_message(message))
        .map(RuntimeMessage::text_content)
        .find(|text| !text.trim().is_empty())
}

/// 收集本轮成功工具产出的可读摘要(用于去敏重做的输入)。
fn gathered_previews(state: &RunState) -> Vec<String> {
    state
        .tool_records
        .iter()
        .filter(|r| r.status == AgentToolStatus::Success)
        .filter_map(|r| {
            r.output
                .as_deref()
                .map(|output| super::execute::limit_tool_text_for_model(
                    &crate::chat::tools::summarize_tool_text(output), r.output_limit,
                ))
                .filter(|p| !p.trim().is_empty())
                .map(|p| format!("【{}】\n{}", r.name, p))
        })
        .collect()
}

/// 去敏 + 精简的恢复输入:仅用「用户问题 + 工具产出摘要 + 中立指令」重做一次合成,
/// 去掉触发审核的完整正文/历史。
fn build_neutral_reduced_messages(state: &RunState, language: &str) -> Vec<RuntimeMessage> {
    let question = last_user_text(&state.runtime_messages).unwrap_or_default();
    let previews = gathered_previews(state).join("\n\n");
    let system = format!(
        "Answer the user's question objectively and neutrally, strictly based on the search snippets below. Only organize and state information already present in the snippets; add no commentary, stance, or outside content. Respond in the language of the user's question unless they request another language. If unclear, use the configured language: {language}."
    );
    let user = format!("User question: {question}\n\nSearch snippets:\n{previews}");
    vec![
        RuntimeMessage::system(system),
        RuntimeMessage::text(ModelRole::User, user),
    ]
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
) -> Result<RecoveredAnswer, String> {
    let config = env.config;
    let failure_message = &failure.message;
    let kind = silent_overflow_aware_kind(env, state, recovery::failure_kind(failure.kind));
    let has_results = !state.tool_records.is_empty();
    // 恢复中枢只在此处被调用一次/次失败,故 already_remediated / overflow_recovery_attempted
    // 都从 false 起算;真正的「只重试一次」守门在各分支内部用本地标志实现。
    Ok(match recovery::decide(kind, has_results, false, false) {
        RecoveryAction::Surface => RecoveredAnswer::fallback(String::new()),
        RecoveryAction::DegradeToGathered => {
            // 结构化留存一份供前端渲染成独立卡片；返回值仍是同样的纯文本，
            // 兼容旧前端 / 外部 CLI（它们只读 content）。
            let degraded = recovery::assemble_degraded_answer(
                &state.tool_records,
                &config.language,
                kind,
                Some(failure_message),
            );
            let text = degraded
                .as_ref()
                .map(|d| d.text.clone())
                .unwrap_or_default();
            state.degraded = degraded;
            RecoveredAnswer::fallback(text)
        }
        RecoveryAction::CompactAndRetry => recover_overflow_compact_and_retry(env, state).await?,
        RecoveryAction::Remediate => recover_remediate(env, state, kind, failure_message).await,
    })
}

/// 把 `Empty` 在**有用量证据**时改判成 `ContextOverflow`（pi `isContextOverflow`
/// case 2/3 的落地，判据见 [`recovery::is_silent_overflow`]）。
///
/// 「模型调用成功但正文为空」有两种成因，处置完全相反：真的没话说 → 重来无意义,
/// 拿工具结果降级；被静默吞掉的超窗请求 → 压缩一次再重发，多半就出来了。
/// `classify` 只有一个空串可看，分不出来；provider 实报的 prompt token 能。
///
/// 只改判 `Empty`：其它 kind 都有明确的错误文案，不该被用量猜测覆盖。
fn silent_overflow_aware_kind(
    env: &LoopEnv<'_>,
    state: &RunState,
    kind: recovery::FailureKind,
) -> recovery::FailureKind {
    if kind != recovery::FailureKind::Empty {
        return kind;
    }
    let Some(usage) = state.last_step_usage.as_ref() else {
        return kind;
    };
    let config = env.config;
    let window = crate::chat::model_metadata::context_window_for_model(
        Some(&config.provider),
        &config.model,
    )
    .0;
    let prompt = super::context_estimate::prompt_tokens(usage, &config.provider.api_format);
    if !recovery::is_silent_overflow(prompt, window) {
        return kind;
    }
    eprintln!(
        "Chat: empty response with prompt {prompt:?} tokens against window {window} — \
         treating as silent context overflow (compact and retry)"
    );
    recovery::FailureKind::ContextOverflow
}

/// CompactAndRetry 执行:压缩一次历史 → 用压缩后的发送视图重发一次合成。/// 成功 → 用其结果;仍失败 → 降级到确定性兜底(对应 decide 的 overflow_recovery_attempted 臂)。
/// 单次守门:本函数只压缩-重试一次,绝不递归,杜绝「压完仍超 → 再压」死循环。
async fn recover_overflow_compact_and_retry(
    env: &LoopEnv<'_>,
    state: &mut RunState,
) -> Result<RecoveredAnswer, String> {
    let config = env.config;
    // 压缩一次(L1 snip → L2 摘要);返回压缩后的发送视图,并已写回 state.runtime_messages。
    let compacted = super::compaction::compact_send_view(env, state, true, &[]).await?;
    if state.compaction_blocked {
        return Ok(RecoveredAnswer::fallback(
            recovery::assemble_results_from_tool_records(
                &state.tool_records,
                &config.language,
                recovery::FailureKind::ContextOverflow,
            ),
        ));
    }
    if config.depth == 0 {
        env.host.context_prepared(&config.provider.id, &config.model, &compacted, &[]);
    }
    // 恢复重试内部有 send_with_retry 多次退避——必须接取消，否则用户点停止后卡到重试耗尽。
    let result = tokio::select! {
        result = config.provider_runtime.message(super::provider_runtime::MessageRequest {
            provider: &config.provider,
            model: &config.model,
            messages: compacted,
            tool_choice: crate::chat::model::ToolChoice::Auto,
            retry_attempts: config.retry_attempts,
            thinking_enabled: config.thinking_enabled,
            thinking_level: config.thinking_level.clone(),
            builtin_web_search: config.builtin_web_search_active(),
            max_output_tokens: config.max_output_tokens,
            conversation_id: &config.conversation_id,
            message_id: &config.message_id,
            label: "Chat synthesis overflow recovery",
        }) => result,
        _ = env.host.control.wait_inactive() => {
            Err(ModelError::with_kind("cancelled", ModelErrorKind::Cancelled))
        }
    };
    // 重试自己的报错也要留给卡片——否则用户只看到"压缩后仍失败"这句空话。
    let mut retry_error: Option<String> = None;
    match result {
        Ok(mut output) => {
            state.merge_usage_with_billing(
                output.usage.clone(),
                output.billing_usage.clone(),
                env,
                true,
            );
            output.text = sanitize_assistant_text_response(&output.text);
            if !output.text.trim().is_empty() {
                state.generated_images.append(&mut output.images);
                return Ok(RecoveredAnswer::from_model(output));
            }
        }
        Err(err) => {
            eprintln!("Chat synthesis overflow compact-and-retry failed: {err}");
            retry_error = Some(err.message);
        }
    }
    let degraded = recovery::assemble_degraded_answer(
        &state.tool_records,
        &config.language,
        recovery::FailureKind::ContextOverflow,
        retry_error.as_deref(),
    );
    let text = degraded
        .as_ref()
        .map(|answer| answer.text.clone())
        .unwrap_or_default();
    state.degraded = degraded;
    Ok(RecoveredAnswer::fallback(text))
}

/// Remediate 执行:用「用户问题 + 工具产出摘要 + 中立指令」去敏精简后重做一次合成。
/// 仍失败 → 确定性兜底(decide 的 already_remediated 臂)。
async fn recover_remediate(
    env: &LoopEnv<'_>,
    state: &mut RunState,
    kind: recovery::FailureKind,
    failure_message: &str,
) -> RecoveredAnswer {
    let config = env.config;
    let reduced = build_neutral_reduced_messages(state, &config.language);
    // 同 recover_overflow_compact_and_retry：恢复重试必须接取消。
    let result = tokio::select! {
        result = config.provider_runtime.message(super::provider_runtime::MessageRequest {
            provider: &config.provider,
            model: &config.model,
            messages: reduced,
            tool_choice: crate::chat::model::ToolChoice::Auto,
            retry_attempts: config.retry_attempts,
            thinking_enabled: config.thinking_enabled,
            thinking_level: config.thinking_level.clone(),
            builtin_web_search: config.builtin_web_search_active(),
            max_output_tokens: config.max_output_tokens,
            conversation_id: &config.conversation_id,
            message_id: &config.message_id,
            label: "Chat synthesis recovery",
        }) => result,
        _ = env.host.control.wait_inactive() => {
            Err(ModelError::with_kind("cancelled", ModelErrorKind::Cancelled))
        }
    };
    if let Ok(mut output) = result {
        state.merge_usage_with_billing(
            output.usage.clone(),
            output.billing_usage.clone(),
            env,
            false,
        );
        output.text = sanitize_assistant_text_response(&output.text);
        if !output.text.trim().is_empty() {
            state.generated_images.append(&mut output.images);
            return RecoveredAnswer::from_model(output);
        }
    }
    let degraded = recovery::assemble_degraded_answer(
        &state.tool_records,
        &config.language,
        kind,
        Some(failure_message),
    );
    let text = degraded
        .as_ref()
        .map(|answer| answer.text.clone())
        .unwrap_or_default();
    state.degraded = degraded;
    RecoveredAnswer::fallback(text)
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
        config.provider.id,
        config.model,
        phase,
        tool_record_count,
        stream.finish_reason.as_ref().map(|reason| reason.raw.as_str()).unwrap_or("unknown"),
        stream.text.chars().count(),
        stream.reasoning.as_deref().map(|value| value.chars().count()).unwrap_or(0),
        stream.tool_calls.len(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn last_user_text_skips_todo_reminders() {
        let messages = vec![
            RuntimeMessage::text(ModelRole::User, "修一下登录页"),
            RuntimeMessage::text(
                ModelRole::User,
                "<todo-reminder>\n1. [pending] x\n</todo-reminder>",
            ),
        ];
        assert_eq!(last_user_text(&messages).as_deref(), Some("修一下登录页"));
    }
}
