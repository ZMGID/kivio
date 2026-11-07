use super::content::AgentToolCall;
use super::history::RuntimeMessage;
use futures::FutureExt;

use crate::tools::ToolDefinition;

use super::error::{AgentAdmissionError, AgentFailure, AgentRunError};
use super::execute::ToolExecutor;
use super::finalize::{
    cancelled_run_result_from_state, finalize_completed, finalize_planning_final, SegmentBuilder,
};
use super::host::AgentHost;
use super::planning::{planning_step, PlanningStepOutcome};
use super::rounds::{run_tool_round, ToolRoundOutcome};
use super::steering::{
    follow_up_pending, inject_follow_up_messages, inject_steering_messages, steering_pending,
};
use super::synthesis::{synthesis_step, SynthesisCompleted, SynthesisFlow};
use super::types::{AgentRunConfig, AgentRunResult};

/// Immutable per-run environment shared by every loop phase.
pub(crate) struct LoopEnv<'a> {
    pub(crate) config: &'a AgentRunConfig<'a>,
    pub(crate) host: &'a super::context::AgentContext<'a>,
    pub(crate) executor: &'a dyn ToolExecutor,
    deadline: Option<tokio::time::Instant>,
}

/// Mutable accumulators threaded through the loop phases.
pub(crate) struct RunState {
    pub(crate) history: super::SessionHistory,
    pub(crate) tools: Vec<ToolDefinition>,
    pub(crate) blocked_tool_calls: Vec<ToolDefinition>,
    pub(crate) generated_api_messages: Vec<RuntimeMessage>,
    pub(crate) tool_records: Vec<AgentToolCall>,
    pub(crate) planning_reasoning_parts: Vec<String>,
    pub(crate) segment_builder: SegmentBuilder,
    pub(crate) step_number: u8,
    pub(crate) model_steps: u32,
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
    pub(crate) usage: Option<crate::model::ModelUsage>,
    pub(crate) cache_pairs: Option<(u64, u64)>,
    /// 本轮**最后一次**模型调用的 usage（真实用量锚点，与累计 `usage` 区分——累计是多步之和、
    /// 会虚高数倍，不能当锚点）。`maybe_compact_send_view` 据此把上下文占用锚定到 provider
    /// 实报值，只对锚点之后新增的消息做字符估算（对齐 pi/opencode 的 ground-truth 口径）。
    /// 压缩发生后清空（消息序列已变，旧锚点失真），下次模型调用重新填充。
    pub(crate) last_step_usage: Option<crate::model::ModelUsage>,
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
    pub(crate) generated_images: Vec<crate::model::GeneratedImageData>,
}

impl RunState {
    pub(crate) fn snapshot(&self) -> super::session::RunSnapshot<'_> {
        super::session::RunSnapshot {
            history: self.history.context(),
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
    pub(crate) fn merge_usage_with_billing(
        &mut self,
        next: Option<crate::model::ModelUsage>,
        billing: Option<crate::model::ModelUsage>,
        env: &LoopEnv<'_>,
        display: bool,
    ) {
        let main_display = display && env.config.report_context;
        if display {
            self.last_step_usage = next.clone();
            self.initial_anchor_valid = false;
        }
        if next.is_none() && main_display {
            env.host.context_unreported();
        }
        let bill = billing.or_else(|| next.clone());
        let Some(bill) = bill else {
            return;
        };
        if main_display {
            crate::model::context_usage::accumulate_cache_pair(
                &mut self.cache_pairs,
                &bill,
                &env.config.api_format,
            );
            env.host.context_cache(self.cache_pairs);
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
        if let Some(used) = crate::model::context_usage::prompt_tokens(&next, api_format)
            .map(|input| input.saturating_add(next.output_tokens.unwrap_or(0)))
        {
            let cache = crate::model::context_usage::combine_cache(
                env.config.initial_cache_usage,
                self.cache_pairs,
            );
            env.host
                .context_usage(used, Some("provider_context_reported"), None, cache);
        } else {
            env.host.context_unreported();
        }
    }
}

/// One request boundary for streamed, collected and recovery calls.
pub(crate) async fn prepare_model_request(
    env: &LoopEnv<'_>,
    state: &mut RunState,
    session: &mut dyn super::session::SessionPolicy,
    call: &mut super::provider_runtime::ModelCall,
) -> Result<(), AgentFailure> {
    if env.host.control.active()
        && env
            .deadline
            .is_some_and(|deadline| tokio::time::Instant::now() >= deadline)
    {
        env.host.control.stop();
        return Err(AgentFailure::LimitExceeded {
            limit: super::error::RunLimit::Timeout,
        });
    }
    let cancelled = || {
        AgentFailure::model(
            "request",
            crate::model::ModelError::with_kind(
                "cancelled",
                crate::model::ModelErrorKind::Cancelled,
            ),
        )
    };
    if !env.host.control.active() {
        return Err(cancelled());
    }
    if env
        .config
        .limits
        .max_model_steps
        .is_some_and(|max| state.model_steps >= max.get())
    {
        return Err(AgentFailure::LimitExceeded {
            limit: super::error::RunLimit::ModelSteps,
        });
    }
    tokio::select! {
        biased;
        _ = env.host.control.wait_inactive() => return Err(cancelled()),
        prepared = session.prepare_request(state.snapshot(), call, env.host.control.clone()) =>
            prepared.map_err(AgentFailure::hook("prepare_request"))?,
    }
    if !env.host.control.active() {
        return Err(cancelled());
    }
    let history = tokio::select! {
        biased;
        _ = env.host.control.wait_inactive() => return Err(cancelled()),
        history = session.transform_context(&call.history, env.host.control.clone()) =>
            history.map_err(AgentFailure::hook("transform_context"))?,
    };
    if !env.host.control.active() {
        return Err(cancelled());
    }
    if env
        .deadline
        .is_some_and(|deadline| tokio::time::Instant::now() >= deadline)
    {
        env.host.control.stop();
        return Err(AgentFailure::LimitExceeded {
            limit: super::error::RunLimit::Timeout,
        });
    }
    call.history = history;
    env.config
        .validate_call(call)
        .map_err(|error| AgentFailure::model("request", error))?;
    state.model_steps = state.model_steps.saturating_add(1);
    Ok(())
}

struct RunGuard<'a> {
    host: &'a super::context::AgentContext<'a>,
    outcome: super::events::AgentRunOutcome,
}
impl Drop for RunGuard<'_> {
    fn drop(&mut self) {
        self.host.control.finish_core();
        self.host
            .events
            .emit(super::events::AgentEventKind::RunEnded {
                outcome: self.outcome.clone(),
            });
        self.host
            .run_ended(self.host.events.identity.conversation_id);
    }
}

struct HookTurnGuard<'a> {
    events: &'a super::events::AgentEvents<'a>,
    round: u32,
    message_open: bool,
}
impl<'a> HookTurnGuard<'a> {
    fn new(events: &'a super::events::AgentEvents<'a>, round: u32) -> Self {
        use super::events::AgentEventKind;
        events.emit(AgentEventKind::StepStarted { round });
        events.emit(AgentEventKind::MessageStarted { round });
        Self {
            events,
            round,
            message_open: true,
        }
    }
    fn end_message(&mut self) {
        if self.message_open {
            self.message_open = false;
            self.events
                .emit(super::events::AgentEventKind::MessageEnded { round: self.round });
        }
    }
}
impl Drop for HookTurnGuard<'_> {
    fn drop(&mut self) {
        self.end_message();
        self.events
            .emit(super::events::AgentEventKind::StepEnded { round: self.round });
    }
}

pub async fn run_agent_loop(
    config: AgentRunConfig<'_>,
    operations: &dyn AgentHost,
    executor: &dyn ToolExecutor,
    session: &mut dyn super::session::SessionPolicy,
    sink: &dyn super::events::AgentEventSink,
    control: super::control::AgentRunHandle<'_>,
) -> Result<AgentRunResult, AgentRunError> {
    use super::events::{AgentEventKind, AgentRunOutcome};
    let identity = control.identity();
    if identity.conversation_id != config.conversation_id
        || identity.run_id != config.run_id
        || identity.message_id != config.message_id
        || identity.generation != config.generation
    {
        return Err(AgentRunError::Admission(
            AgentAdmissionError::IdentityMismatch,
        ));
    }
    control
        .begin_core()
        .map_err(|error| AgentRunError::Admission(AgentAdmissionError::Control(error)))?;
    let host = super::context::AgentContext::new(operations, sink, control);
    host.events.emit(AgentEventKind::RunStarted);
    let mut guard = RunGuard {
        host: &host,
        outcome: AgentRunOutcome::Dropped,
    };
    let deadline = config
        .limits
        .timeout
        .and_then(|timeout| tokio::time::Instant::now().checked_add(timeout));
    let running = run_agent_loop_inner(config, &host, executor, session, deadline);
    tokio::pin!(running);
    let result = if let Some(deadline) = deadline {
        tokio::select! {
            biased;
            result = &mut running => result,
            _ = tokio::time::sleep_until(deadline) => {
                let timed_out = host.control.active();
                if timed_out { host.control.stop(); }
                let result = running.await;
                if timed_out { timeout_result(result) } else { result }
            }
        }
    } else {
        running.await
    };
    let result = if host.control.active()
        && deadline.is_some_and(|deadline| tokio::time::Instant::now() >= deadline)
    {
        host.control.stop();
        timeout_result(result)
    } else {
        result
    };
    guard.outcome = match &result {
        Ok(result) => match &result.status {
            super::types::AgentRunStatus::Cancelled => AgentRunOutcome::Cancelled,
            super::types::AgentRunStatus::Failed(error) => AgentRunOutcome::Failed(error.clone()),
            super::types::AgentRunStatus::Recovered => AgentRunOutcome::Recovered,
            super::types::AgentRunStatus::CompactionBlocked => AgentRunOutcome::CompactionBlocked,
            _ => AgentRunOutcome::Completed,
        },
        Err(AgentRunError::Failed { cause, .. }) => AgentRunOutcome::Failed(cause.clone()),
        Err(AgentRunError::Admission(_)) => unreachable!("admission precedes RunStarted"),
    };
    result
}

fn timeout_result(
    result: Result<AgentRunResult, AgentRunError>,
) -> Result<AgentRunResult, AgentRunError> {
    let cause = AgentFailure::LimitExceeded {
        limit: super::error::RunLimit::Timeout,
    };
    let mut partial = match result {
        Ok(result) => result,
        Err(AgentRunError::Failed { partial, .. }) => *partial,
        Err(error) => return Err(error),
    };
    partial.status = super::types::AgentRunStatus::Failed(cause.clone());
    Err(AgentRunError::Failed {
        cause,
        partial: Box::new(partial),
    })
}

enum Answer {
    Planning(crate::model::GenerateOutput),
    Synthesis(SynthesisCompleted),
    Recovered(AgentRunResult),
}

impl Answer {
    fn message(&self) -> RuntimeMessage {
        match self {
            Self::Planning(output) => RuntimeMessage::model(output.replay_message()),
            Self::Recovered(result) => {
                RuntimeMessage::model(result.final_message.clone().unwrap_or_else(|| {
                    super::stop::final_assistant_message(
                        &result.content,
                        result.reasoning.as_deref(),
                    )
                }))
            }
            Self::Synthesis(completed) => {
                completed.generated_message.clone().unwrap_or_else(|| {
                    RuntimeMessage::model(completed.replay.clone().unwrap_or_else(|| {
                        super::stop::final_assistant_message(
                            &completed.response,
                            completed.response_reasoning.as_deref(),
                        )
                    }))
                })
            }
        }
    }

    fn absorb(self, state: &mut RunState) {
        let message = match self {
            Self::Recovered(result) => {
                RuntimeMessage::model(result.final_message.unwrap_or_else(|| {
                    super::stop::final_assistant_message(
                        &result.content,
                        result.reasoning.as_deref(),
                    )
                }))
            }
            Self::Planning(output) => RuntimeMessage::model(output.replay_message()),
            Self::Synthesis(completed) => {
                if let Some(reasoning) = completed.response_reasoning.as_deref() {
                    if !reasoning.trim().is_empty() {
                        state.planning_reasoning_parts.push(reasoning.to_owned());
                    }
                }
                completed.generated_message.unwrap_or_else(|| {
                    RuntimeMessage::model(completed.replay.unwrap_or_else(|| {
                        super::stop::final_assistant_message(
                            &completed.response,
                            completed.response_reasoning.as_deref(),
                        )
                    }))
                })
            }
        };
        state.history.push(message.clone());
        state.generated_api_messages.push(message);
        state.runtime_len_at_last_call = state.history.context().len();
    }

    fn finish(self, state: &mut RunState) -> Result<AgentRunResult, AgentFailure> {
        match self {
            Self::Planning(output) => {
                finalize_planning_final(state, output).map_err(AgentFailure::InvalidOutput)
            }
            Self::Synthesis(completed) => Ok(finalize_completed(state, completed)),
            Self::Recovered(result) => Ok(result),
        }
    }

    fn cancel(self, state: &mut RunState) -> Result<AgentRunResult, AgentFailure> {
        let mut result = self.finish(state)?;
        result.status = super::types::AgentRunStatus::Cancelled;
        Ok(record_final_message(result, state))
    }
}

// Early recovery builders own the accumulated facts; restore them before scheduling another turn.
fn restore_completed_result(state: &mut RunState, result: &AgentRunResult) {
    state.tool_records = result.tool_records.clone();
    state.generated_api_messages = result.generated_history.clone();
    state
        .segment_builder
        .append_existing_segments(result.segments.clone());
}

enum AnswerBoundary {
    Continue,
    Finish(Answer),
    Cancelled(AgentRunResult),
}

async fn admit_step(
    env: &LoopEnv<'_>,
    state: &mut RunState,
    round: u32,
    session: &mut dyn super::session::SessionPolicy,
) -> Result<(), AgentFailure> {
    inject_steering_messages(env, state, round, session).await?;
    if !env.host.control.active() {
        return Ok(());
    }
    let incoming = env
        .host
        .checkpoint_runtime(
            &env.config.conversation_id,
            &env.config.run_id,
            &state.history,
            false,
        )
        .await
        .map_err(AgentFailure::host("checkpoint_runtime"))?;
    state
        .history
        .extend(incoming.iter().map(|input| input.message.clone()));
    state.generated_api_messages.extend(
        incoming
            .into_iter()
            .filter(|input| input.persistence == super::host::InputPersistence::Append)
            .map(|input| input.message),
    );
    if env.host.control.active() {
        let inputs = session.before_step(state.snapshot());
        if env.host.control.active() {
            state.history.extend(inputs.iter().cloned());
            state.generated_api_messages.extend(inputs);
        }
    }
    Ok(())
}

async fn answer_boundary(
    env: &LoopEnv<'_>,
    state: &mut RunState,
    round: u32,
    session: &mut dyn super::session::SessionPolicy,
    answer: Answer,
) -> Result<AnswerBoundary, AgentFailure> {
    if !env.host.control.active() {
        return Ok(AnswerBoundary::Cancelled(answer.cancel(state)?));
    }
    let mut history = state.history.context().to_vec();
    history.push(answer.message());
    let decision = tokio::select! {
        biased;
        _ = env.host.control.wait_inactive() => return Ok(AnswerBoundary::Cancelled(answer.cancel(state)?)),
        decision = std::panic::AssertUnwindSafe(async {
            session.finish_turn(super::session::RunSnapshot {
                history: &history,
                tools: &state.tools,
                tool_records: &state.tool_records,
            }, env.host.control.clone()).await
        }).catch_unwind() => decision.unwrap_or_else(|_| Err("finish_turn panicked".into())),
    };
    if !env.host.control.active() {
        return Ok(AnswerBoundary::Cancelled(answer.cancel(state)?));
    }
    match decision {
        Err(message) => {
            answer.absorb(state);
            return Err(AgentFailure::hook("finish_turn")(message));
        }
        Ok(super::session::TurnDecision::Continue(messages)) => {
            answer.absorb(state);
            state.history.extend(messages.iter().cloned());
            state.generated_api_messages.extend(messages);
            return Ok(AnswerBoundary::Continue);
        }
        Ok(super::session::TurnDecision::Stop) => return Ok(AnswerBoundary::Finish(answer)),
        Ok(super::session::TurnDecision::Default) => {}
    }
    let incoming = match env
        .host
        .checkpoint_runtime(
            &env.config.conversation_id,
            &env.config.run_id,
            &state.history,
            true,
        )
        .await
    {
        Ok(incoming) => incoming,
        Err(message) => {
            answer.absorb(state);
            return Err(AgentFailure::host("checkpoint_runtime")(message));
        }
    };
    if !env.host.control.active() {
        let mut result = answer.cancel(state)?;
        if result.generated_history.is_empty()
            && incoming
                .iter()
                .any(|input| input.persistence == super::host::InputPersistence::Append)
        {
            result
                .generated_history
                .extend(result.final_message.clone().map(RuntimeMessage::model));
        }
        state
            .history
            .extend(incoming.iter().map(|input| input.message.clone()));
        result.generated_history.extend(
            incoming
                .into_iter()
                .filter(|input| input.persistence == super::host::InputPersistence::Append)
                .map(|input| input.message),
        );
        return Ok(AnswerBoundary::Cancelled(result));
    }
    if !incoming.is_empty() {
        answer.absorb(state);
        state
            .history
            .extend(incoming.iter().map(|input| input.message.clone()));
        state.generated_api_messages.extend(
            incoming
                .into_iter()
                .filter(|input| input.persistence == super::host::InputPersistence::Append)
                .map(|input| input.message),
        );
        return Ok(AnswerBoundary::Continue);
    }
    if steering_pending(env, state) {
        answer.absorb(state);
        return Ok(AnswerBoundary::Continue);
    }
    if follow_up_pending(env, state) {
        answer.absorb(state);
        inject_follow_up_messages(env, state, round, session).await?;
        return Ok(AnswerBoundary::Continue);
    }
    let child_activity = match env
        .host
        .wait_for_child_results(
            &env.config.conversation_id,
            &env.config.run_id,
            env.config.generation,
        )
        .await
    {
        Ok(activity) => activity,
        Err(message) => {
            answer.absorb(state);
            return Err(AgentFailure::host("wait_for_child_results")(message));
        }
    };
    if !env.host.control.active() {
        return Ok(AnswerBoundary::Cancelled(answer.cancel(state)?));
    }
    if child_activity {
        answer.absorb(state);
        if follow_up_pending(env, state) {
            inject_follow_up_messages(env, state, round, session).await?;
        }
        return Ok(AnswerBoundary::Continue);
    }
    Ok(AnswerBoundary::Finish(answer))
}

async fn run_agent_loop_inner(
    mut config: AgentRunConfig<'_>,
    host: &super::context::AgentContext<'_>,
    executor: &dyn ToolExecutor,
    session: &mut dyn super::session::SessionPolicy,
    deadline: Option<tokio::time::Instant>,
) -> Result<AgentRunResult, AgentRunError> {
    let mut state = RunState {
        history: std::mem::take(&mut config.history),
        tools: std::mem::take(&mut config.tools),
        blocked_tool_calls: std::mem::take(&mut config.blocked_tool_calls),
        generated_api_messages: Vec::new(),
        tool_records: Vec::new(),
        planning_reasoning_parts: Vec::new(),
        segment_builder: SegmentBuilder::new(),
        step_number: 0,
        model_steps: 0,
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
    let result: Result<AgentRunResult, AgentFailure> = std::panic::AssertUnwindSafe(async {
        let tool_loop_ran = !state.tools.is_empty();
        let inputs = tokio::select! {
            biased;
            _ = host.control.wait_inactive() => Vec::new(),
            inputs = session.start(state.snapshot(), host.control.clone()) =>
                inputs.map_err(AgentFailure::hook("start"))?,
        };
        if !host.control.active() {
            return Ok(cancelled_run_result_from_state(&mut state));
        }
        state.history.extend(inputs.iter().cloned());
        state.generated_api_messages.extend(inputs);
        let env = LoopEnv {
            config: &config,
            host,
            executor,
            deadline,
        };
        let mut round = 0u32;
        let planning_final = if tool_loop_ran {
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
                    return Ok(result);
                }

                // 本轮的 turn_start / message_start 由 guard 构造时发；turn_end / message_end
                // 由它的 Drop 补齐，所以下面任何一条 return / continue / `?` 都不会漏配对。
                let mut turn = HookTurnGuard::new(&host.events, round);

                admit_step(&env, &mut state, round, session).await?;
                if !host.control.active() {
                    let result = cancelled_run_result_from_state(&mut state);
                    return Ok(result);
                }

                let planned = match planning_step(&env, &mut state, round, session).await? {
                    PlanningStepOutcome::FinalAnswer(message) => {
                        match answer_boundary(
                            &env,
                            &mut state,
                            round,
                            session,
                            Answer::Planning(message),
                        )
                        .await?
                        {
                            AnswerBoundary::Continue => continue,
                            AnswerBoundary::Finish(answer) => break Some(answer),
                            AnswerBoundary::Cancelled(result) => return Ok(result),
                        }
                    }
                    PlanningStepOutcome::ToolsUnsupported => break None,
                    PlanningStepOutcome::RetryWithTools => continue,
                    PlanningStepOutcome::RetryEmptyResponse => continue,
                    PlanningStepOutcome::DraftFailed(mut result) => {
                        let mut records = std::mem::take(&mut state.tool_records);
                        records.append(&mut result.tool_records);
                        result.tool_records = records;
                        return Ok(record_final_message(result, &mut state));
                    }
                    PlanningStepOutcome::Recovered(result) => {
                        if result.status != super::types::AgentRunStatus::Recovered {
                            return Ok(record_final_message(result, &mut state));
                        }
                        restore_completed_result(&mut state, &result);
                        match answer_boundary(
                            &env,
                            &mut state,
                            round,
                            session,
                            Answer::Recovered(result),
                        )
                        .await?
                        {
                            AnswerBoundary::Continue => continue,
                            AnswerBoundary::Finish(answer) => break Some(answer),
                            AnswerBoundary::Cancelled(result) => return Ok(result),
                        }
                    }
                    PlanningStepOutcome::Cancelled(result) => {
                        return Ok(record_final_message(result, &mut state));
                    }
                    PlanningStepOutcome::ToolCalls(planned) => planned,
                };
                // 消息（含工具调用决策）到此产出完毕，但本轮要等工具跑完才算结束。
                turn.end_message();

                match run_tool_round(&env, &mut state, round, planned).await {
                    ToolRoundOutcome::Continue => {
                        state.completed_tool_rounds += 1;
                    }
                    ToolRoundOutcome::Failed(failure) => return Err(failure),
                    ToolRoundOutcome::Cancelled(result) => {
                        return Ok(record_final_message(result, &mut state));
                    }
                }
                let decision = tokio::select! {
                    biased;
                    _ = host.control.wait_inactive() => super::session::TurnDecision::Default,
                    decision = session.finish_turn(state.snapshot(), host.control.clone()) =>
                        decision.map_err(AgentFailure::hook("finish_turn"))?,
                };
                if host.control.active() {
                    match decision {
                        super::session::TurnDecision::Default => {}
                        super::session::TurnDecision::Continue(messages) => {
                            state.history.extend(messages.iter().cloned());
                            state.generated_api_messages.extend(messages);
                        }
                        super::session::TurnDecision::Stop => {
                            let mut result = cancelled_run_result_from_state(&mut state);
                            result.status = super::types::AgentRunStatus::Completed;
                            return Ok(result);
                        }
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
                    &state.history,
                )
                .await;
                let incoming = host
                    .checkpoint_runtime(
                        &config.conversation_id,
                        &config.run_id,
                        &state.history,
                        false,
                    )
                    .await
                    .map_err(AgentFailure::host("checkpoint_runtime"))?;
                state
                    .history
                    .extend(incoming.iter().map(|input| input.message.clone()));
                state.generated_api_messages.extend(
                    incoming
                        .into_iter()
                        .filter(|input| input.persistence == super::host::InputPersistence::Append)
                        .map(|input| input.message),
                );
            }
        } else {
            None
        };

        // A managed tool may finish cleanup after cancellation. Recheck before
        // synthesis.
        if !host.control.active() {
            let result = cancelled_run_result_from_state(&mut state);
            return Ok(result);
        }
        let incoming = host
            .checkpoint_runtime(
                &config.conversation_id,
                &config.run_id,
                &state.history,
                true,
            )
            .await
            .map_err(AgentFailure::host("checkpoint_runtime"))?;
        state
            .history
            .extend(incoming.iter().map(|input| input.message.clone()));
        state.generated_api_messages.extend(
            incoming
                .into_iter()
                .filter(|input| input.persistence == super::host::InputPersistence::Append)
                .map(|input| input.message),
        );

        if let Some(prompt) = state.fallback_prompt.take() {
            state.history.set_system_prompt(&prompt.system);
            if let Some(message) = prompt.context {
                state.history.push(message.clone());
                state.generated_api_messages.push(message);
            }
        }

        if let Some(message) = planning_final {
            return message
                .finish(&mut state)
                .map(|result| record_final_message(result, &mut state));
        }

        loop {
            if !host.control.active() {
                let result = cancelled_run_result_from_state(&mut state);
                return Ok(result);
            }
            round = round.saturating_add(1);
            let _turn = HookTurnGuard::new(&host.events, round);
            admit_step(&env, &mut state, round, session).await?;
            if !host.control.active() {
                let result = cancelled_run_result_from_state(&mut state);
                return Ok(result);
            }
            let completed = match synthesis_step(&env, &mut state, session).await? {
                SynthesisFlow::Early(result) => {
                    if result.status != super::types::AgentRunStatus::Recovered {
                        return Ok(record_final_message(result, &mut state));
                    }
                    restore_completed_result(&mut state, &result);
                    match answer_boundary(
                        &env,
                        &mut state,
                        round,
                        session,
                        Answer::Recovered(result),
                    )
                    .await?
                    {
                        AnswerBoundary::Continue => continue,
                        AnswerBoundary::Finish(answer) => {
                            return answer
                                .finish(&mut state)
                                .map(|result| record_final_message(result, &mut state))
                        }
                        AnswerBoundary::Cancelled(result) => return Ok(result),
                    }
                }
                SynthesisFlow::CancelledState => {
                    let result = cancelled_run_result_from_state(&mut state);
                    return Ok(result);
                }
                SynthesisFlow::Completed(completed) => completed,
            };
            match answer_boundary(
                &env,
                &mut state,
                round,
                session,
                Answer::Synthesis(completed),
            )
            .await?
            {
                AnswerBoundary::Continue => continue,
                AnswerBoundary::Finish(answer) => {
                    return answer
                        .finish(&mut state)
                        .map(|result| record_final_message(result, &mut state))
                }
                AnswerBoundary::Cancelled(result) => return Ok(result),
            }
        }
    })
    .catch_unwind()
    .await
    .unwrap_or_else(|_| {
        Err(AgentFailure::Host {
            operation: "run_agent_loop.panic",
            message: "Agent run panicked".into(),
        })
    });
    result
        .map(|result| attach_run_state(result, &mut state))
        .or_else(|cause| {
            let mut partial = cancelled_run_result_from_state(&mut state);
            if !host.control.active() && !matches!(cause, AgentFailure::LimitExceeded { .. }) {
                return Ok(attach_run_state(partial, &mut state));
            }
            partial.status = super::types::AgentRunStatus::Failed(cause.clone());
            let partial = Box::new(attach_run_state(partial, &mut state));
            Err(AgentRunError::Failed { cause, partial })
        })
}

fn record_final_message(result: AgentRunResult, state: &mut RunState) -> AgentRunResult {
    if let Some(final_message) = result.final_message.clone().or_else(|| {
        (!result.content.is_empty() || result.reasoning.is_some()).then(|| {
            super::stop::final_assistant_message(&result.content, result.reasoning.as_deref())
        })
    }) {
        state.history.push(RuntimeMessage::model(final_message));
    }
    result
}

fn attach_run_state(mut result: AgentRunResult, state: &mut RunState) -> AgentRunResult {
    result.history = std::mem::take(&mut state.history);
    result.usage = state.usage.take();
    result.cache_pairs = state.cache_pairs.take();
    result.last_step_usage = state.last_step_usage.take();
    result.compacted = state.compacted;
    result.images.append(&mut state.generated_images);
    result
        .pending_steering
        .extend(state.pending_steering.drain(..));
    result
        .pending_follow_up
        .extend(state.pending_follow_up.drain(..));
    result
}
