use super::error::AgentFailure;
use super::{
    history::RuntimeMessage,
    loop_::{LoopEnv, RunState},
    session::{ContextReason, ContextSnapshot, ContextUpdate, SessionPolicy},
};
use crate::model::ModelTool;

pub(crate) async fn maybe_compact_send_view(
    env: &LoopEnv<'_>,
    state: &mut RunState,
    tools: &[ModelTool],
    session: &mut dyn SessionPolicy,
) -> Result<Vec<RuntimeMessage>, AgentFailure> {
    compact_send_view(env, state, false, tools, session).await
}

pub(crate) async fn compact_send_view(
    env: &LoopEnv<'_>,
    state: &mut RunState,
    force: bool,
    tools: &[ModelTool],
    session: &mut dyn SessionPolicy,
) -> Result<Vec<RuntimeMessage>, AgentFailure> {
    let preparation = session.prepare_context(
        if force {
            ContextReason::Overflow
        } else {
            ContextReason::BeforeRequest
        },
        ContextSnapshot {
            history: state.history.context(),
            tools,
            last_step_usage: state.last_step_usage.as_ref(),
            runtime_len_at_last_call: state.runtime_len_at_last_call,
            initial_anchor_valid: state.initial_anchor_valid,
            completed_tool_rounds: state.completed_tool_rounds,
        },
        env.host.control.clone(),
        env.host,
    );
    let prepared = tokio::select! {
        biased;
        _ = env.host.control.wait_inactive() => {
            session.context_rejected("interrupted", env.host);
            return Ok(state.history.context().to_vec());
        }
        prepared = preparation => prepared.map_err(AgentFailure::hook("prepare_context"))?,
    };
    if !env.host.control.active() {
        session.context_rejected("interrupted", env.host);
        return Ok(state.history.context().to_vec());
    }
    state.compaction_blocked |= prepared.blocked;
    match prepared.update {
        None => {}
        Some(ContextUpdate::SendView(history)) => {
            if let Err(error) = state.history.reset_context(history) {
                session.context_rejected("failed", env.host);
                return Err(AgentFailure::hook("prepare_context")(error));
            }
            super::session::PolicyProgress::context_reset(env.host, None);
            state.last_step_usage = None;
            state.initial_anchor_valid = false;
        }
        Some(ContextUpdate::Compacted(plan)) => {
            if let Err(error) = state.history.validate_compaction(&plan) {
                session.context_rejected("failed", env.host);
                return Err(AgentFailure::hook("prepare_context")(error));
            }
            let incoming = match env
                .host
                .checkpoint_runtime(
                    &env.config.conversation_id,
                    &env.config.run_id,
                    &state.history,
                    false,
                )
                .await
            {
                Ok(incoming) => incoming,
                Err(error) => {
                    session.context_rejected("failed", env.host);
                    return Err(AgentFailure::host("checkpoint_runtime")(error));
                }
            };
            if !env.host.control.active() {
                session.context_rejected("interrupted", env.host);
                state
                    .history
                    .extend(incoming.iter().map(|input| input.message.clone()));
                state.generated_api_messages.extend(
                    incoming
                        .into_iter()
                        .filter(|input| input.persistence == super::host::InputPersistence::Append)
                        .map(|input| input.message),
                );
                return Ok(state.history.context().to_vec());
            }
            state.generated_api_messages.extend(
                incoming
                    .iter()
                    .filter(|input| input.persistence == super::host::InputPersistence::Append)
                    .map(|input| input.message.clone()),
            );
            state
                .history
                .compact(plan)
                .map_err(AgentFailure::hook("prepare_context"))?;
            state
                .history
                .extend(incoming.into_iter().map(|input| input.message));
            state.compacted = true;
            state.last_step_usage = None;
            state.initial_anchor_valid = false;
            let (inputs, blocked) = tokio::select! {
                biased;
                _ = env.host.control.wait_inactive() => (Vec::new(), false),
                applied = session.context_applied(state.history.context(), env.host.control.clone(), env.host) => applied,
            };
            if !env.host.control.active() {
                return Ok(state.history.context().to_vec());
            }
            state.history.extend(inputs);
            state.compaction_blocked |= blocked;
        }
    }
    Ok(state.history.context().to_vec())
}
