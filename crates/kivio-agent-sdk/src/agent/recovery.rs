use crate::model::{FinishKind, ModelError, ModelErrorKind};

#[derive(Clone, Copy)]
pub struct RecoverySnapshot<'a> {
    pub history: &'a [super::history::RuntimeMessage],
    pub tool_records: &'a [super::content::AgentToolCall],
    pub last_step_usage: Option<&'a crate::model::ModelUsage>,
}

pub enum RecoveryInput {
    CompactCurrent,
    Reduced(Vec<super::history::RuntimeMessage>),
}

pub enum RecoveryDecision {
    Surface,
    Complete(String),
    Retry(RecoveryInput),
}

pub enum RecoveryFailure<'a> {
    Blocked,
    Empty,
    Error(&'a ModelError),
}

#[derive(Clone, Copy)]
pub enum FallbackReason {
    EmptySynthesis,
    FailedSynthesis,
    FailedPlanning,
}

pub(crate) fn snapshot(state: &super::loop_::RunState) -> RecoverySnapshot<'_> {
    RecoverySnapshot {
        history: &state.runtime_messages,
        tool_records: &state.tool_records,
        last_step_usage: state.last_step_usage.as_ref(),
    }
}

#[derive(Clone)]
pub(crate) struct StreamRequest<'a> {
    pub call: super::provider_runtime::ModelCall,
    pub policy: super::types::AgentStreamPolicy,
    pub text_segment: Option<super::content::ContentBlock>,
    pub reasoning_segment: Option<super::content::ContentBlock>,
    pub tool_draft_tracker: Option<super::stream::ToolCallDraftTracker>,
    pub web_search_tracker: Option<super::stream::WebSearchCardTracker>,
    pub speculation: Option<(
        &'a dyn super::execute::ToolExecutor,
        &'a super::execute::ToolExecutionContext<'a>,
    )>,
}

pub(crate) fn is_recoverable_text_length(
    output: &crate::model::GenerateOutput,
    max_output_tokens: u32,
) -> bool {
    !output.cancelled
        && output
            .finish_reason
            .as_ref()
            .is_some_and(|reason| reason.kind == FinishKind::Length)
        && !output.text.trim().is_empty()
        && output.tool_calls.is_empty()
        && output.images.is_empty()
        && max_output_tokens > 0
        && output
            .usage
            .as_ref()
            .and_then(|usage| usage.output_tokens)
            .is_some_and(|tokens| tokens < u64::from(max_output_tokens))
}

/// Pi's recoverable length case: output stopped below the intended output cap.
/// The failed response never enters model history. Both planning and plain answers
/// share one compact-and-retry owner; transport retries remain in the provider.
pub(crate) async fn stream_with_truncation_recovery(
    env: &super::loop_::LoopEnv<'_>,
    state: &mut super::loop_::RunState,
    mut request: StreamRequest<'_>,
    session: &mut dyn super::session::SessionPolicy,
) -> Result<crate::model::GenerateOutput, crate::model::ModelError> {
    let mut attempted = false;
    loop {
        let result = super::planning::stream_model_call(
            env.config.provider_runtime,
            env.host,
            request.call.clone(),
            request.policy,
            request.text_segment.clone(),
            request.reasoning_segment.clone(),
            request.tool_draft_tracker.clone(),
            request.web_search_tracker.clone(),
            request.speculation,
        )
        .await;
        let output = match result {
            Ok(output) => output,
            Err(error) if attempted => {
                return Err(ModelError::with_kind(
                    format!("Truncated response recovery failed: {error}"),
                    ModelErrorKind::TruncationRecoveryFailed,
                ))
            }
            Err(error) => return Err(error),
        };
        let recoverable = is_recoverable_text_length(&output, request.call.options.max_tokens);
        if !recoverable {
            return Ok(output);
        }
        // Account for the discarded attempt, but keep it out of persisted/replayed text.
        state.merge_usage_with_billing(
            output.usage.clone(),
            output.billing_usage.clone(),
            env,
            true,
        );
        if attempted {
            return Err(ModelError::with_kind("Truncated response recovery failed after one compact-and-retry attempt. Please reduce context or switch models.", ModelErrorKind::TruncationRecoveryFailed));
        }
        attempted = true;
        let compacted =
            super::compaction::compact_send_view(env, state, true, &request.call.tools, session)
                .await
                .map_err(ModelError::new)?;
        if !env.host.control.active() {
            return Ok(crate::model::GenerateOutput::cancelled(String::new(), None));
        }
        if state.compaction_blocked || compacted == request.call.history {
            return Err(ModelError::with_kind("Truncated response recovery failed: context could not be compacted. Please reduce context or switch models.", ModelErrorKind::TruncationRecoveryFailed));
        }
        request.call.history = compacted;
    }
}
