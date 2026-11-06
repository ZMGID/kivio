use super::content::{AgentToolCall};
use super::history::RuntimeMessage;


use crate::chat::tools::ToolDefinition;

use super::execute::ToolExecutor;
use super::finalize::{
    cancelled_run_result_from_state, finalize_completed, finalize_planning_final, SegmentBuilder,
};
use super::host::AgentHost;
use super::planning::{planning_step, PlanningStepOutcome};
use super::rounds::{run_tool_round, ToolRoundOutcome};
use super::steering::{
    absorb_final_answer, follow_up_pending, inject_follow_up_messages, inject_steering_messages,
    steering_pending,
};
use super::stop::patch_system_message;
use super::synthesis::{synthesis_step, SynthesisFlow};
use super::types::{AgentRunConfig, AgentRunResult};

/// Immutable per-run environment shared by every loop phase.
pub(crate) struct LoopEnv<'a> {
    pub(crate) config: &'a AgentRunConfig<'a>,
    pub(crate) host: &'a super::context::AgentContext<'a>,
    pub(crate) executor: &'a dyn ToolExecutor,
}

/// Mutable accumulators threaded through the loop phases.
pub(crate) struct RunState {
    pub(crate) runtime_messages: Vec<RuntimeMessage>,
    pub(crate) tools: Vec<ToolDefinition>,
    pub(crate) blocked_tool_calls: Vec<ToolDefinition>,
    pub(crate) generated_api_messages: Vec<RuntimeMessage>,
    pub(crate) tool_records: Vec<AgentToolCall>,
    pub(crate) planning_reasoning_parts: Vec<String>,
    pub(crate) segment_builder: SegmentBuilder,
    pub(crate) step_number: u8,
    pub(crate) provider_tools_unsupported: bool,
    pub(crate) fallback_prompt: Option<super::session::PromptUpdate>,
    /// 空响应重试守门（一次）：抽风网关会间歇性返回 200 + 空正文（无文本无工具调用）。
    /// 第一次遇到时 planning 返回 `RetryEmptyResponse` 原地重试；已重试过则照旧走
    /// FinalAnswer → finalize 报 "empty assistant response"。
    pub(crate) planning_empty_retried: bool,
    /// 待注入的用户插话本地队列（对齐 pi `PendingMessageQueue`）：信箱一次取空后暂存在
    /// 这里，`inject_steering_messages` 每个轮次边界只弹一条（one-at-a-time），剩余的由
    /// 后续边界与 FinalAnswer 边界的 `steering_pending` 检查保证送达。
    pub(crate) pending_steering: std::collections::VecDeque<super::steering::SteeringMessage>,
    /// 原生 follow-up 本地队列。只在 FinalAnswer 边界取信箱、注入，不进轮首——那是 steer。
    pub(crate) pending_follow_up: std::collections::VecDeque<super::steering::SteeringMessage>,
    /// 本轮全部模型调用（规划/合成/压缩摘要）的 usage 累计；provider 不报则保持 None。
    pub(crate) usage: Option<crate::chat::model::ModelUsage>,
    pub(crate) cache_pairs: Option<(u64, u64)>,
    /// 本轮**最后一次**模型调用的 usage（真实用量锚点，与累计 `usage` 区分——累计是多步之和、
    /// 会虚高数倍，不能当锚点）。`maybe_compact_send_view` 据此把上下文占用锚定到 provider
    /// 实报值，只对锚点之后新增的消息做字符估算（对齐 pi/opencode 的 ground-truth 口径）。
    /// 压缩发生后清空（消息序列已变，旧锚点失真），下次模型调用重新填充。
    pub(crate) last_step_usage: Option<crate::chat::model::ModelUsage>,
    /// 记录锚点**响应 push 之后** `runtime_messages` 的长度——`runtime_messages[该值..]` =
    /// 锚点响应之后新增的消息（工具结果等），即 trailing 增量。在 `rounds.rs` push 完 assistant
    /// 响应后设置（而非 merge_usage 时），使 trailing 严格「响应之后」、不与锚点 output 双算。
    pub(crate) runtime_len_at_last_call: usize,
    /// `config.initial_anchor_*`（来自上一轮落盘 usage）是否仍可用：run 首次压缩检查前为 true；
    /// 一旦发生压缩即失效（回落纯估算，直到本轮模型调用产生新的 `last_step_usage`）。
    pub(crate) initial_anchor_valid: bool,
    pub(crate) compacted: bool,
    pub(crate) completed_tool_rounds: u32,
    pub(crate) compaction_blocked: bool,
    /// 模型原生生成的图片（Gemini native image gen，任务 07-24）：跨轮累积每次答案调用
    /// （planning/synthesis）产出的 `GenerateOutput.images`，run 结束时由 `attach_usage`
    /// 挂到 `AgentRunResult.images` → reply 落成 assistant 消息级 artifacts。
    pub(crate) generated_images: Vec<crate::chat::model::GeneratedImageData>,

}

impl RunState {
    pub(crate) fn snapshot(&self) -> super::session::RunSnapshot<'_> {
        super::session::RunSnapshot {
            history: &self.runtime_messages,
            tools: &self.tools,
            tool_records: &self.tool_records,
        }
    }

    /// 把单次模型调用的 usage 累加进本轮总账；缺失实报时清除当前锚点。
    /// 同时把这次调用记为**真实用量锚点**（`last_step_usage`）——累计 `usage` 是多步之和不能当
    /// 锚点，锚点必须是单次调用的 usage。`runtime_len_at_last_call`（trailing 切点）不在这里设，
    /// 而在 `rounds.rs` push 完该次响应后设——保证 trailing = 锚点响应**之后**新增（对齐 pi、避免
    /// 与锚点里的 output 双算）。
    /// 使用独立精简输入的 recovery 必须清除锚点，它的实报不代表完整运行上下文。
    /// `display` is false for a reduced recovery prompt: its bill still counts,
    /// but it is not the context meter or the budget anchor.
    pub(crate) fn merge_usage(
        &mut self,
        next: Option<crate::chat::model::ModelUsage>,
        env: &LoopEnv<'_>,
        display: bool,
    ) {
        self.merge_usage_with_billing(next, None, env, display);
    }

    pub(crate) fn merge_usage_with_billing(
        &mut self,
        next: Option<crate::chat::model::ModelUsage>,
        billing: Option<crate::chat::model::ModelUsage>,
        env: &LoopEnv<'_>,
        display: bool,
    ) {
        let main_display = display && env.config.report_context;
        if display {
            self.last_step_usage = next.clone();
            self.initial_anchor_valid = false;
        }
        if next.is_none() && main_display {
            env.host
                .context_unreported();
        }
        let bill = billing.or_else(|| next.clone());
        let Some(bill) = bill else {
            return;
        };
        if main_display {
            crate::chat::model::context_usage::accumulate_cache_pair(
                &mut self.cache_pairs,
                &bill,
                &env.config.api_format,
            );
            env.host
                .context_cache(self.cache_pairs);
        }
        let total = self.usage.get_or_insert_with(Default::default);
        let add = |slot: &mut Option<u64>, value: Option<u64>| {
            if let Some(value) = value {
                *slot = Some(slot.unwrap_or(0).saturating_add(value));
            }
        };
        add(&mut total.input_tokens, bill.input_tokens);
        add(&mut total.output_tokens, bill.output_tokens);
        add(&mut total.total_tokens, bill.total_tokens);
        add(&mut total.cached_input_tokens, bill.cached_input_tokens);
        add(
            &mut total.cache_creation_input_tokens,
            bill.cache_creation_input_tokens,
        );
        add(&mut total.reasoning_tokens, bill.reasoning_tokens);
        if !main_display {
            return;
        }
        let Some(next) = next else {
            return;
        };
        let api_format = env.config.api_format.as_str();
        if let Some(used) = crate::chat::model::context_usage::prompt_tokens(&next, api_format)
            .map(|input| input.saturating_add(next.output_tokens.unwrap_or(0)))
        {
            let cache = crate::chat::model::context_usage::combine_cache(
                env.config.initial_cache_usage,
                self.cache_pairs,
            );
            env.host.context_usage(used, Some("provider_context_reported"), None, cache);
        } else {
            env.host
                .context_unreported();
        }
    }
}

struct RunGuard<'a> {
    host: &'a super::context::AgentContext<'a>,
    outcome: super::events::AgentRunOutcome,
}
impl Drop for RunGuard<'_> {
    fn drop(&mut self) {
        self.host.control.finish_core();
        self.host.events.emit(super::events::AgentEventKind::RunEnded { outcome: self.outcome.clone() });
        self.host.run_ended(self.host.events.identity.conversation_id);
    }
}

struct HookTurnGuard<'a> { events: &'a super::events::AgentEvents<'a>, round: u32, message_open: bool }
impl<'a> HookTurnGuard<'a> {
    fn new(events: &'a super::events::AgentEvents<'a>, round: u32) -> Self {
        use super::events::AgentEventKind;
        events.emit(AgentEventKind::StepStarted { round });
        events.emit(AgentEventKind::MessageStarted { round });
        Self { events, round, message_open: true }
    }
    fn end_message(&mut self) {
        if self.message_open {
            self.message_open = false;
            self.events.emit(super::events::AgentEventKind::MessageEnded { round: self.round });
        }
    }
}
impl Drop for HookTurnGuard<'_> {
    fn drop(&mut self) {
        self.end_message();
        self.events.emit(super::events::AgentEventKind::StepEnded { round: self.round });
    }
}

pub async fn run_agent_loop(
    config: AgentRunConfig<'_>,
    operations: &dyn AgentHost,
    executor: &dyn ToolExecutor,
    session: &mut dyn super::session::SessionPolicy,
    sink: &dyn super::events::AgentEventSink,
    control: super::control::AgentRunHandle<'_>,
) -> Result<AgentRunResult, String> {
    use super::events::{AgentEventKind, AgentEvents, AgentRunOutcome};
    let identity = control.identity();
    if identity.conversation_id != config.conversation_id || identity.run_id != config.run_id
        || identity.message_id != config.message_id || identity.generation != config.generation {
        return Err("agent run control identity does not match configuration".into());
    }
    control.begin_core().map_err(|error| error.to_string())?;
    let host = super::context::AgentContext { host: operations, events: AgentEvents::new(identity, sink, operations.hooks().map(|hooks| hooks as &dyn super::events::AgentEventSink), control.invocation_id()), control };
    host.events.emit(AgentEventKind::RunStarted);
    let mut guard = RunGuard { host: &host, outcome: AgentRunOutcome::Dropped };
    let result = run_agent_loop_inner(config, &host, executor, session).await;
    guard.outcome = match &result {
        Ok(result) => match &result.status {
            super::types::AgentRunStatus::Cancelled => AgentRunOutcome::Cancelled,
            super::types::AgentRunStatus::Failed(error) => AgentRunOutcome::Failed(error.clone()),
            _ => AgentRunOutcome::Completed,
        },
        Err(_) if !host.control.active() => AgentRunOutcome::Cancelled,
        Err(error) => AgentRunOutcome::Failed(error.clone()),
    };
    result
}

async fn run_agent_loop_inner(
    mut config: AgentRunConfig<'_>,
    host: &super::context::AgentContext<'_>,
    executor: &dyn ToolExecutor,
    session: &mut dyn super::session::SessionPolicy,
) -> Result<AgentRunResult, String> {
    let mut state = RunState {
        runtime_messages: std::mem::take(&mut config.runtime_messages),
        tools: std::mem::take(&mut config.tools),
        blocked_tool_calls: std::mem::take(&mut config.blocked_tool_calls),
        generated_api_messages: Vec::new(),
        tool_records: Vec::new(),
        planning_reasoning_parts: Vec::new(),
        segment_builder: SegmentBuilder::new(),
        step_number: 0,
        provider_tools_unsupported: false,
        fallback_prompt: None,
        planning_empty_retried: false,
        pending_steering: std::collections::VecDeque::new(),
        pending_follow_up: std::collections::VecDeque::new(),
        usage: None,
        cache_pairs: None,
        last_step_usage: None,
        runtime_len_at_last_call: 0,
        initial_anchor_valid: true,
        compacted: false,
        completed_tool_rounds: 0,
        compaction_blocked: false,
        generated_images: Vec::new(),
    };
    let tool_loop_ran = !state.tools.is_empty();
    let inputs = session.start(state.snapshot(), host.control.clone()).await?;
    state.runtime_messages.extend(inputs.iter().cloned());
    state.generated_api_messages.extend(inputs);
    let env = LoopEnv {
        config: &config,
        host,
        executor,
    };
    let planning_final = if tool_loop_ran {
        let mut round = 0u32;
        loop {
            round = round.saturating_add(1);
            state.step_number = state.step_number.saturating_add(1);
            if !host.control.active() {
                // Cancelled at the loop top (before this round's planning call).
                // Preserve whatever previous rounds already accumulated
                // (tool_records / segments / api_messages) by ending with
                // Ok(cancelled_result) instead of a bare Err("cancelled") — the
                // latter skipped persistence and dropped the whole turn.
                let result = cancelled_run_result_from_state(&mut state);
                return Ok(attach_usage(result, &mut state));
            }

            // 本轮的 turn_start / message_start 由 guard 构造时发；turn_end / message_end
            // 由它的 Drop 补齐，所以下面任何一条 return / continue / `?` 都不会漏配对。
            let mut turn = HookTurnGuard::new(&host.events, round);

            // 用户在生成期间点了「立刻引导」：轮次边界是唯一安全的注入点（上一轮的工具结果
            // 已经落进历史，本轮还没开始调模型）。信箱为空时零开销。
            inject_steering_messages(&env, &mut state, round, session).await?;

            let incoming = host
                .checkpoint_runtime(
                    &config.conversation_id,
                    &config.run_id,
                    &state.runtime_messages,
                    false,
                )
                .await?;
            state.runtime_messages.extend(incoming.iter().map(|input| input.message.clone()));
            state.generated_api_messages.extend(
                incoming
                    .into_iter()
                    .filter(|input| input.persistence == super::host::InputPersistence::Append).map(|input| input.message),
            );
            let inputs = session.before_step(state.snapshot());
            state.runtime_messages.extend(inputs.iter().cloned());
            state.generated_api_messages.extend(inputs);

            let planned = match planning_step(&env, &mut state, round, session).await? {
                PlanningStepOutcome::FinalAnswer(message) => {
                    let incoming = host
                        .checkpoint_runtime(
                            &config.conversation_id,
                            &config.run_id,
                            &state.runtime_messages,
                            true,
                        )
                        .await?;
                    if !incoming.is_empty() {
                        absorb_final_answer(&mut state, message);
                        state.runtime_messages.extend(incoming.iter().map(|input| input.message.clone()));
                        state
                            .generated_api_messages
                            .extend(incoming.into_iter().filter(|input| input.persistence == super::host::InputPersistence::Append).map(|input| input.message));
                        continue;
                    }
                    // 立刻引导：终答时还有没送达的插话 ⇒ 吸收终答、下一轮轮首注入。
                    // 原生 follow-up：等终答，不打断工具循环；吸收后在本边界注入一条再续跑。
                    if steering_pending(&env, &mut state) {
                        absorb_final_answer(&mut state, message);
                        continue;
                    }
                    if follow_up_pending(&env, &mut state) {
                        absorb_final_answer(&mut state, message);
                        inject_follow_up_messages(&env, &mut state, round, session).await?;
                        continue;
                    }
                    let child_activity = host
                        .wait_for_child_results(
                            &config.conversation_id,
                            &config.run_id,
                            config.generation,
                        )
                        .await?;
                    if !host.control.active() {
                        let result = cancelled_run_result_from_state(&mut state);
                        return Ok(attach_usage(result, &mut state));
                    }
                    if child_activity {
                        absorb_final_answer(&mut state, message);
                        // Follow-ups can arrive while waiting, after the check
                        // above. Inject them before the next model request.
                        if follow_up_pending(&env, &mut state) {
                            inject_follow_up_messages(&env, &mut state, round, session).await?;
                        }
                        continue;
                    }
                    if let Some(reminder) = session.before_finish(state.snapshot()) {
                        absorb_final_answer(&mut state, message);
                        state.runtime_messages.push(reminder.clone());
                        state.generated_api_messages.push(reminder);
                        continue;
                    }
                    break Some(message);
                }
                PlanningStepOutcome::ToolsUnsupported => break None,
                PlanningStepOutcome::RetryWithTools => continue,
                PlanningStepOutcome::RetryEmptyResponse => continue,
                PlanningStepOutcome::DraftFailed(result) => {
                    return Ok(attach_usage(result, &mut state))
                }
                PlanningStepOutcome::Recovered(result) => {
                    return Ok(attach_usage(result, &mut state));
                }
                PlanningStepOutcome::Cancelled(result) => {
                    return Ok(attach_usage(result, &mut state));
                }
                PlanningStepOutcome::ToolCalls(planned) => planned,
            };
            // 消息（含工具调用决策）到此产出完毕，但本轮要等工具跑完才算结束。
            turn.end_message();

            match run_tool_round(&env, &mut state, round, planned).await {
                ToolRoundOutcome::Continue => {
                    state.completed_tool_rounds += 1;
                }
                ToolRoundOutcome::Cancelled(result) => {
                    return Ok(attach_usage(result, &mut state));
                }
            }
            // 本轮到此完整结束（turn_end 随 guard 落地），再落盘快照。
            drop(turn);
            // Crash-safety checkpoint: persist a best-effort snapshot of the
            // partial assistant message after each completed tool round. The
            // final assistant message is otherwise written only once, after this
            // loop returns (`push_assistant_message`); if the process dies mid-run
            // the whole turn — including files just written by tools this round —
            // vanishes. Persisting here keeps the work recoverable on next load.
            // The final write replaces this draft (same `message_id`), so there
            // is no duplication on the normal success path.
            host.persist_partial_assistant(
                &config.conversation_id,
                &config.message_id,
                &state.tool_records,
                state.segment_builder.segments(),
                &state.generated_api_messages,
            )
            .await;
            let incoming = host
                .checkpoint_runtime(
                    &config.conversation_id,
                    &config.run_id,
                    &state.runtime_messages,
                    false,
                )
                .await?;
            state.runtime_messages.extend(incoming.iter().map(|input| input.message.clone()));
            state.generated_api_messages.extend(
                incoming
                    .into_iter()
                    .filter(|input| input.persistence == super::host::InputPersistence::Append).map(|input| input.message),
            );
        }
    } else {
        None
    };

    // A managed tool may finish cleanup after cancellation. Recheck before
    // synthesis.
    if !host.control.active() {
        let result = cancelled_run_result_from_state(&mut state);
        return Ok(attach_usage(result, &mut state));
    }
    let incoming = host
        .checkpoint_runtime(
            &config.conversation_id,
            &config.run_id,
            &state.runtime_messages,
            true,
        )
        .await?;
    state.runtime_messages.extend(incoming.iter().map(|input| input.message.clone()));
    state.generated_api_messages.extend(
        incoming
            .into_iter()
            .filter(|input| input.persistence == super::host::InputPersistence::Append).map(|input| input.message),
    );

    if let Some(prompt) = state.fallback_prompt.take() {
        patch_system_message(&mut state.runtime_messages, &prompt.system);
        if let Some(message) = prompt.context {
            state.runtime_messages.push(message.clone());
            state.generated_api_messages.push(message);
        }
    }

    if let Some(message) = planning_final {
        return finalize_planning_final(&mut state, message)
            .map(|result| attach_usage(result, &mut state));
    }

    // 无工具会话（工具全关 / provider 不支持）整段跳过上面的循环，一个 turn/message 事件
    // 都不会发——只剩 agent_start/agent_end。这里的 synthesis 就是那一「轮」，补上配对
    // （对齐 LiveAgent 的 runTextConversationTurn 也走 startTurn/endTurn）。工具循环跑过
    // 的情况下它自己的 guard 已经配好了，不能再发一次。
    let _text_turn = (!tool_loop_ran).then(|| HookTurnGuard::new(&host.events, 1));

    match synthesis_step(&env, &mut state, session).await? {
        SynthesisFlow::Early(result) => Ok(attach_usage(result, &mut state)),
        SynthesisFlow::Completed(completed) => {
            let result = finalize_completed(&env, &mut state, completed);
            Ok(attach_usage(result, &mut state))
        }
    }
}

/// 把本轮累计的 provider usage 挂到运行结果上（finalize 构造器们不感知 usage）。
/// 同时：本轮若发生过 L2 压缩，把压缩后的完整历史 +最终 assistant 回答回传到
/// `compacted_history`，供跨轮调用方替换其累积历史（finalize 构造器们也不感知压缩）。
fn attach_usage(mut result: AgentRunResult, state: &mut RunState) -> AgentRunResult {
    result.usage = state.usage.take();
    result.cache_pairs = state.cache_pairs.take();
    result.last_step_usage = state.last_step_usage.take();
    result.history = std::mem::take(&mut state.runtime_messages);
    if let Some(final_message) = result.final_message.clone().or_else(|| {
        (!result.content.is_empty() || result.reasoning.is_some()).then(||
            super::stop::final_assistant_message(&result.content, result.reasoning.as_deref()))
    }) {
        result.history.push(RuntimeMessage::model(final_message));
    }
    result.compacted = state.compacted;
    result.images = std::mem::take(&mut state.generated_images);
    result
}

#[cfg(test)]
pub(crate) use super::finalize::{
    cancelled_tool_round_run_result, tool_planning_failed_run_result,
};
#[cfg(test)]
pub(crate) use super::rounds::{
    tool_message,
};
#[cfg(test)]
pub(crate) use super::stream::{AgentStreamSink, ToolCallDraftTracker};
#[cfg(test)]
pub(crate) use crate::chat::model::PendingToolCall;
#[cfg(test)]
pub(crate) use crate::settings::Settings;

#[cfg(test)]
#[path = "loop_tests.rs"]
pub(crate) mod tests;
