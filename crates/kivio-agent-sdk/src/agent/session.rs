use super::{content::AgentToolCall, history::RuntimeMessage};
use super::{control::AgentRunHandle, events::CompactionFact, host::AgentHostFuture};
use crate::model::{ModelTool, ModelUsage};
use crate::tools::ToolDefinition;

pub type PolicyFuture<'a, T> = AgentHostFuture<'a, T>;

#[derive(Clone, Copy)]
pub enum ContextReason {
    BeforeRequest,
    Overflow,
}

pub struct ContextSnapshot<'a> {
    pub history: &'a [RuntimeMessage],
    pub tools: &'a [ModelTool],
    pub last_step_usage: Option<&'a ModelUsage>,
    pub runtime_len_at_last_call: usize,
    pub initial_anchor_valid: bool,
    pub completed_tool_rounds: u32,
}

#[derive(Debug, Clone)]
pub struct CompactionPlan {
    /// Exact range in the ContextSnapshot that produced this plan.
    pub covered: std::ops::Range<usize>,
    pub replacement: Vec<RuntimeMessage>,
}

pub enum ContextUpdate {
    SendView(Vec<RuntimeMessage>),
    Compacted(CompactionPlan),
}

#[derive(Default)]
pub struct ContextPreparation {
    pub update: Option<ContextUpdate>,
    pub blocked: bool,
}

pub trait PolicyProgress: Send + Sync {
    fn compaction(&self, phase: &str, trigger: Option<&str>, boundary: Option<CompactionFact<'_>>);
    fn context_reset(&self, window: Option<u64>);
}

#[derive(Clone, Copy)]
pub struct RunSnapshot<'a> {
    pub history: &'a [RuntimeMessage],
    pub tools: &'a [ToolDefinition],
    pub tool_records: &'a [AgentToolCall],
}

pub struct PromptUpdate {
    pub system: String,
    pub context: Option<RuntimeMessage>,
}

pub enum ToolsRejected {
    Retry(Vec<ToolDefinition>),
    Plain(Option<PromptUpdate>),
}

/// Scheduling decision after one assistant message and its tool results.
#[derive(Debug, Default)]
pub enum TurnDecision {
    #[default]
    Default,
    Stop,
    Continue(Vec<RuntimeMessage>),
}

pub trait SessionPolicy: Send {
    fn start<'a>(
        &'a mut self,
        _run: RunSnapshot<'a>,
        _control: AgentRunHandle<'a>,
    ) -> PolicyFuture<'a, Result<Vec<RuntimeMessage>, String>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    fn input<'a>(
        &'a mut self,
        _prompt: &'a str,
        _control: AgentRunHandle<'a>,
    ) -> PolicyFuture<'a, Result<Vec<RuntimeMessage>, String>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    fn tools_rejected(&mut self, _run: RunSnapshot<'_>) -> ToolsRejected {
        ToolsRejected::Plain(None)
    }

    fn recover(
        &mut self,
        _run: super::recovery::RecoverySnapshot<'_>,
        _error: &crate::model::ModelError,
    ) -> super::recovery::RecoveryDecision {
        super::recovery::RecoveryDecision::Surface
    }

    fn recovery_failed(
        &mut self,
        _run: super::recovery::RecoverySnapshot<'_>,
        _failure: super::recovery::RecoveryFailure<'_>,
    ) -> String {
        String::new()
    }

    fn context_blocked(&mut self, _run: super::recovery::RecoverySnapshot<'_>) -> String {
        String::new()
    }

    fn fallback(&self, _reason: super::recovery::FallbackReason) -> String {
        String::new()
    }

    fn prepare_context<'a>(
        &'a mut self,
        _reason: ContextReason,
        _run: ContextSnapshot<'a>,
        _control: AgentRunHandle<'a>,
        _progress: &'a dyn PolicyProgress,
    ) -> PolicyFuture<'a, Result<ContextPreparation, String>> {
        Box::pin(async { Ok(ContextPreparation::default()) })
    }

    fn context_applied<'a>(
        &'a mut self,
        _history: &'a [RuntimeMessage],
        _control: AgentRunHandle<'a>,
        _progress: &'a dyn PolicyProgress,
    ) -> PolicyFuture<'a, (Vec<RuntimeMessage>, bool)> {
        Box::pin(async { (Vec::new(), false) })
    }

    fn context_rejected(&mut self, _phase: &str, _progress: &dyn PolicyProgress) {}

    /// Appends canonical context after input admission, before preparing a request view.
    fn before_step(&mut self, _run: RunSnapshot<'_>) -> Vec<RuntimeMessage> {
        Vec::new()
    }

    /// Edits the next request without changing canonical history.
    fn prepare_request<'a>(
        &'a mut self,
        _run: RunSnapshot<'a>,
        _request: &'a mut super::provider_runtime::ModelCall,
        _control: AgentRunHandle<'a>,
    ) -> PolicyFuture<'a, Result<(), String>> {
        Box::pin(async { Ok(()) })
    }

    /// Returns the temporary context sent to the model. The returned view is not persisted.
    fn transform_context<'a>(
        &'a mut self,
        history: &'a [RuntimeMessage],
        _control: AgentRunHandle<'a>,
    ) -> PolicyFuture<'a, Result<Vec<RuntimeMessage>, String>> {
        Box::pin(async move { Ok(history.to_vec()) })
    }

    /// Runs once after each complete assistant/tool turn, including tool rounds.
    /// Default follows normal scheduling; Continue appends messages and requests another turn.
    fn finish_turn<'a>(
        &'a mut self,
        _run: RunSnapshot<'a>,
        _control: AgentRunHandle<'a>,
    ) -> PolicyFuture<'a, Result<TurnDecision, String>> {
        Box::pin(async { Ok(TurnDecision::Default) })
    }
}

pub struct NoSessionPolicy;
impl SessionPolicy for NoSessionPolicy {}
