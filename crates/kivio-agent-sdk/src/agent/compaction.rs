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
) -> Result<Vec<RuntimeMessage>, String> {
    compact_send_view(env, state, false, tools, session).await
}

pub(crate) async fn compact_send_view(
    env: &LoopEnv<'_>,
    state: &mut RunState,
    force: bool,
    tools: &[ModelTool],
    session: &mut dyn SessionPolicy,
) -> Result<Vec<RuntimeMessage>, String> {
    let prepared = session
        .prepare_context(
            if force {
                ContextReason::Overflow
            } else {
                ContextReason::BeforeRequest
            },
            ContextSnapshot {
                history: &state.runtime_messages,
                tools,
                last_step_usage: state.last_step_usage.as_ref(),
                runtime_len_at_last_call: state.runtime_len_at_last_call,
                initial_anchor_valid: state.initial_anchor_valid,
                completed_tool_rounds: state.completed_tool_rounds,
            },
            env.host.control.clone(),
            env.host,
        )
        .await?;
    state.compaction_blocked |= prepared.blocked;
    match prepared.update {
        None => {}
        Some(ContextUpdate::SendView(history)) => {
            super::session::PolicyProgress::context_reset(env.host, None);
            state.runtime_messages = history;
            state.last_step_usage = None;
            state.initial_anchor_valid = false;
        }
        Some(ContextUpdate::Compacted(mut history)) => {
            let incoming = match env
                .host
                .checkpoint_runtime(
                    &env.config.conversation_id,
                    &env.config.run_id,
                    &state.runtime_messages,
                    false,
                )
                .await
            {
                Ok(incoming) => incoming,
                Err(error) => {
                    session.context_rejected("failed", env.host);
                    return Err(error);
                }
            };
            if !env.host.control.active() {
                session.context_rejected("interrupted", env.host);
                state
                    .runtime_messages
                    .extend(incoming.iter().map(|input| input.message.clone()));
                state.generated_api_messages.extend(
                    incoming
                        .into_iter()
                        .filter(|input| input.persistence == super::host::InputPersistence::Append)
                        .map(|input| input.message),
                );
                return Ok(state.runtime_messages.clone());
            }
            state.generated_api_messages.extend(
                incoming
                    .iter()
                    .filter(|input| input.persistence == super::host::InputPersistence::Append)
                    .map(|input| input.message.clone()),
            );
            history.extend(incoming.into_iter().map(|input| input.message));
            state.runtime_messages = history;
            state.compacted = true;
            state.last_step_usage = None;
            state.initial_anchor_valid = false;
            let (inputs, blocked) = session
                .context_applied(&state.runtime_messages, env.host.control.clone(), env.host)
                .await;
            state.runtime_messages.extend(inputs);
            state.compaction_blocked |= blocked;
        }
    }
    Ok(state.runtime_messages.clone())
}
