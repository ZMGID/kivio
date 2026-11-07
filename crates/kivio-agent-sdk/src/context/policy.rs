use super::*;
use crate::agent::{
    control::AgentRunHandle,
    history::{export_history, RuntimeMessage},
    provider_runtime::ModelRuntime,
    session::{
        ContextPreparation, ContextReason, ContextSnapshot, ContextUpdate, PolicyFuture,
        PolicyProgress, RunSnapshot, SessionPolicy,
    },
};
use std::sync::Arc;

#[derive(Default)]
pub struct BudgetState {
    pub failures: u32,
    pub initial_anchor_total_tokens: Option<u64>,
    pub initial_anchor_trailing_estimate: usize,
    last_compacted_round: Option<u32>,
    rapid_refills: u32,
}
#[derive(Clone, Copy)]
pub struct BudgetDecision {
    pub summarize: bool,
    pub blocked: bool,
    pub before: usize,
    pub budget: usize,
    pub trigger: &'static str,
    pub rapid_refills: u32,
}
impl BudgetState {
    pub fn new(
        failures: u32,
        initial_anchor_total_tokens: Option<u64>,
        initial_anchor_trailing_estimate: usize,
    ) -> Self {
        Self {
            failures,
            initial_anchor_total_tokens,
            initial_anchor_trailing_estimate,
            ..Self::default()
        }
    }

    pub fn assess(
        &self,
        reason: ContextReason,
        run: &ContextSnapshot<'_>,
        window: usize,
        max_output: u32,
        threshold: Option<usize>,
        api_format: &str,
        full: usize,
        pruned: bool,
    ) -> BudgetDecision {
        let window = if window == 0 { 200_000 } else { window };
        let hard_limit = window.saturating_sub(max_output as usize);
        let budget = threshold
            .unwrap_or_else(|| request_context_budget(window, max_output))
            .min(hard_limit);
        let hard_pressure = full > hard_limit;
        let force = matches!(reason, ContextReason::Overflow);
        let (anchor, trailing) = if pruned {
            (None, 0)
        } else if let Some(usage) = run.last_step_usage {
            (
                crate::model::context_usage::anchor_total_tokens(usage, api_format),
                run.history[run.runtime_len_at_last_call.min(run.history.len())..]
                    .iter()
                    .map(estimate_runtime_message_tokens)
                    .sum(),
            )
        } else if run.initial_anchor_valid {
            (
                self.initial_anchor_total_tokens,
                self.initial_anchor_trailing_estimate,
            )
        } else {
            (None, 0)
        };
        let before = crate::model::context_usage::effective_context_tokens(anchor, trailing, full)
            .0
            .max(full);
        let rapid_refills = if self
            .last_compacted_round
            .is_some_and(|round| run.completed_tool_rounds.saturating_sub(round) < 3)
        {
            self.rapid_refills.saturating_add(1)
        } else {
            0
        };
        let summarize = (force || before >= budget)
            && (force || hard_pressure || self.failures < 3)
            && has_compactable_history(run.history)
            && (force || rapid_refills < 3);
        BudgetDecision {
            summarize,
            blocked: hard_pressure || (!force && rapid_refills >= 3 && before >= budget),
            before,
            budget,
            trigger: if force || hard_pressure {
                "reactive"
            } else {
                "agent_loop"
            },
            rapid_refills,
        }
    }
    pub fn accepts(&self, before: usize, after: usize, budget: usize) -> bool {
        after < before && after <= budget.saturating_mul(9) / 10
    }
    pub fn failed(&mut self) {
        self.failures = self.failures.saturating_add(1);
    }
    pub fn applied(&mut self, round: u32, rapid_refills: u32) {
        self.failures = 0;
        self.initial_anchor_total_tokens = None;
        self.initial_anchor_trailing_estimate = 0;
        self.last_compacted_round = Some(round);
        self.rapid_refills = rapid_refills;
    }
}

/// Ready-to-use session policy. Model selection and persistence remain with the host.
pub struct BudgetContextPolicy {
    runtime: Arc<dyn ModelRuntime>,
    /// Main request model used for budget estimates. Defaults to the summary model.
    pub model: String,
    pub summary: SummaryOptions,
    pub threshold: Option<usize>,
    pub window: usize,
    pub max_output_tokens: u32,
    pub api_format: String,
    pub state: BudgetState,
    pub projection: Option<Arc<dyn SummaryProjection>>,
    candidate: Option<(u32, u32)>,
    tool_schema_tokens: usize,
    started: bool,
}
impl BudgetContextPolicy {
    pub fn new(model: Arc<dyn ModelRuntime>, summary: SummaryOptions) -> Self {
        Self {
            window: summary.window,
            max_output_tokens: summary.options.max_tokens,
            model: summary.model.clone(),
            runtime: model,
            summary,
            threshold: None,
            api_format: String::new(),
            state: BudgetState::default(),
            projection: None,
            candidate: None,
            tool_schema_tokens: 0,
            started: false,
        }
    }
    fn estimate(&self, history: &[RuntimeMessage]) -> usize {
        let messages = export_history(history);
        self.projection.as_ref().map_or_else(
            || estimate_request_tokens(&messages, &self.model),
            |p| p.estimate_request_tokens(&messages, &self.model),
        )
    }
}
impl SessionPolicy for BudgetContextPolicy {
    fn start<'a>(
        &'a mut self,
        _run: RunSnapshot<'a>,
        _control: AgentRunHandle<'a>,
    ) -> PolicyFuture<'a, Result<Vec<RuntimeMessage>, String>> {
        if self.started {
            self.state.initial_anchor_total_tokens = None;
            self.state.initial_anchor_trailing_estimate = 0;
        }
        self.started = true;
        self.state.last_compacted_round = None;
        self.state.rapid_refills = 0;
        self.candidate = None;
        Box::pin(async { Ok(Vec::new()) })
    }

    fn transform_context<'a>(
        &'a mut self,
        history: &'a [RuntimeMessage],
        _control: AgentRunHandle<'a>,
    ) -> PolicyFuture<'a, Result<Vec<RuntimeMessage>, String>> {
        Box::pin(async move {
            let mut view = history.to_vec();
            prune_runtime_images(&mut view, IMAGE_BYTES_BUDGET);
            Ok(view)
        })
    }
    fn prepare_context<'a>(
        &'a mut self,
        reason: ContextReason,
        run: ContextSnapshot<'a>,
        control: AgentRunHandle<'a>,
        progress: &'a dyn PolicyProgress,
    ) -> PolicyFuture<'a, Result<ContextPreparation, String>> {
        Box::pin(async move {
            let mut view = run.history.to_vec();
            let pruned = prune_runtime_images(&mut view, IMAGE_BYTES_BUDGET) > 0;
            let schemas: usize = run
                .tools
                .iter()
                .map(|tool| estimate_value_tokens(&tool.to_openai_tool()))
                .sum();
            let full = self.estimate(&view).saturating_add(schemas);
            self.tool_schema_tokens = schemas;
            let decision = self.state.assess(
                reason,
                &run,
                self.window,
                self.max_output_tokens,
                self.threshold,
                &self.api_format,
                full,
                pruned,
            );
            let mut result = ContextPreparation {
                update: None,
                blocked: control.active() && decision.blocked,
            };
            if !decision.summarize {
                return Ok(result);
            }
            progress.compaction("started", Some(decision.trigger), None);
            match summarize_history(
                self.runtime.as_ref(),
                &self.summary,
                &view,
                run.tools,
                Some(Box::pin(control.wait_inactive())),
                Some(progress),
                self.projection.as_deref(),
            )
            .await
            {
                CompactOutcome::Compacted(plan, _) => {
                    let mut projected = run.history.to_vec();
                    projected.splice(plan.covered.clone(), plan.replacement.clone());
                    prune_runtime_images(&mut projected, IMAGE_BYTES_BUDGET);
                    let after = self.estimate(&projected).saturating_add(schemas);
                    if self.state.accepts(full, after, decision.budget) {
                        self.candidate = Some((run.completed_tool_rounds, decision.rapid_refills));
                        result.update = Some(ContextUpdate::Compacted(plan));
                        result.blocked = false;
                    } else {
                        self.state.failed();
                        progress.compaction("failed", Some(decision.trigger), None);
                    }
                }
                CompactOutcome::Failed => {
                    self.state.failed();
                    progress.compaction("failed", Some(decision.trigger), None);
                }
                CompactOutcome::Cancelled => {
                    progress.compaction("interrupted", Some(decision.trigger), None);
                    result.blocked = false;
                }
                CompactOutcome::Skipped => {
                    progress.compaction("skipped", Some(decision.trigger), None)
                }
            }
            result.blocked &= control.active();
            Ok(result)
        })
    }
    fn context_applied<'a>(
        &'a mut self,
        history: &'a [RuntimeMessage],
        _control: AgentRunHandle<'a>,
        progress: &'a dyn PolicyProgress,
    ) -> PolicyFuture<'a, (Vec<RuntimeMessage>, bool)> {
        Box::pin(async move {
            if let Some((round, refills)) = self.candidate.take() {
                self.state.applied(round, refills);
                progress.context_reset(Some(self.window as u64));
                progress.compaction("completed", None, None);
            }
            let mut view = history.to_vec();
            prune_runtime_images(&mut view, IMAGE_BYTES_BUDGET);
            let window = if self.window == 0 {
                200_000
            } else {
                self.window
            };
            let blocked = self.estimate(&view).saturating_add(self.tool_schema_tokens)
                > window.saturating_sub(self.max_output_tokens as usize);
            (Vec::new(), blocked)
        })
    }
    fn context_rejected(&mut self, phase: &str, progress: &dyn PolicyProgress) {
        if self.candidate.take().is_some() {
            progress.compaction(phase, None, None);
        }
    }
}
