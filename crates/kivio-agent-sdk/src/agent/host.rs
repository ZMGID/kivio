use super::content::{AgentToolCall, ContentBlock};
use std::{future::Future, pin::Pin};

use super::events::AgentEventSink;

pub type AgentHostFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Owns streamed task launches; dropping it discards any that were not adopted.
pub trait SpeculativeTaskScope: Send + Sync {
    fn update(&self, call: &str, raw: &str);
    fn reconcile_call(&self, call: &str, args: Option<&serde_json::Value>);
    fn reconcile(&self, final_calls: &[(&str, &serde_json::Value)]);
    fn discard(&self);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputPersistence {
    Append,
    AlreadyPersisted,
}

#[derive(Debug, Clone)]
pub struct SessionInput {
    pub message: super::history::RuntimeMessage,
    pub persistence: InputPersistence,
}

impl SessionInput {
    pub fn append(message: super::history::RuntimeMessage) -> Self {
        Self {
            message,
            persistence: InputPersistence::Append,
        }
    }
}

pub trait AgentHost: Send + Sync {
    /// Final persistence owned by the convenience Session; direct loop callers commit themselves.
    fn commit_run<'a>(
        &'a self,
        _identity: super::events::AgentRunIdentity<'a>,
        _result: &'a super::types::AgentRunResult,
    ) -> AgentHostFuture<'a, Result<(), String>> {
        Box::pin(async { Ok(()) })
    }
    /// Hosts may require an in-flight tool to finish before cancellation returns.
    fn requires_tool_completion(&self) -> bool {
        false
    }
    fn run_ended(&self, _conversation_id: &str) {}
    /// At an answer boundary, a child host waits for its own jobs; top-level hosts return false.
    fn wait_for_child_results<'a>(
        &'a self,
        _conversation_id: &'a str,
        _run_id: &'a str,
        _generation: u64,
    ) -> AgentHostFuture<'a, Result<bool, String>> {
        Box::pin(async { Ok(false) })
    }
    /// Persist the full session facts, including inactive branches, and receive new messages.
    /// Before compaction this sees the original facts; an error prevents replacement.
    fn checkpoint_runtime<'a>(
        &'a self,
        _conversation_id: &'a str,
        _run_id: &'a str,
        _history: &'a super::session_history::SessionHistory,
        _finishing: bool,
    ) -> AgentHostFuture<'a, Result<Vec<SessionInput>, String>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    /// Persist a best-effort draft after a completed tool round. `api_messages`
    /// contains completed assistant tool calls and results for presentation compatibility.
    /// `history` contains all session facts; before dispatch its active path may end
    /// with pending tool calls, while completed-round drafts contain their results.
    /// This checkpoint does not make external tool effects transactional or safe
    /// to execute again. The default does not persist drafts.
    fn persist_partial_assistant<'a>(
        &'a self,
        _conversation_id: &'a str,
        _message_id: &'a str,
        _tool_records: &'a [AgentToolCall],
        _segments: &'a [ContentBlock],
        _api_messages: &'a [super::history::RuntimeMessage],
        _history: &'a super::session_history::SessionHistory,
    ) -> AgentHostFuture<'a, ()> {
        Box::pin(async {})
    }

    /// Optional lifecycle observer, independent of authorization and cancellation.
    fn hooks(&self) -> Option<&dyn AgentEventSink> {
        None
    }
}
