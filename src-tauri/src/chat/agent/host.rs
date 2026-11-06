use super::content::{ContentBlock, AgentToolCall};
use std::{future::Future, pin::Pin};

use crate::chat::ask_user::{AskUserPromptPayload, AskUserResponseResult};
use crate::chat::hooks::HookDispatcher;

use super::execute::ToolExecutionContext;

pub type AgentHostFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Owns streamed task launches; dropping it discards any that were not adopted.
pub trait SpeculativeTaskScope: Send + Sync {
    fn update(&self, call: &str, raw: &str);
    fn reconcile_call(&self, call: &str, args: Option<&serde_json::Value>);
    fn reconcile(&self, final_calls: &[(&str, &serde_json::Value)]);
    fn discard(&self);
}

pub trait AgentHost: Send + Sync {
    fn compaction_threshold(&self, _context_window: usize) -> Option<usize> {
        None
    }

    /// Parallel answer arms cannot share an interactive approval mailbox safely.
    fn supports_interactive_approval(&self) -> bool {
        true
    }

    fn start_speculation(
        &self,
        _settings: &crate::settings::Settings,
        _tools: &[crate::mcp::ChatToolDefinition],
        _context: &ToolExecutionContext<'_>,
    ) -> Option<Box<dyn SpeculativeTaskScope>> {
        None
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
    /// Persist the supplied history when needed and return newly received messages.
    /// Before compaction this sees the original history; an error prevents replacement.
    fn checkpoint_runtime<'a>(
        &'a self,
        _conversation_id: &'a str,
        _run_id: &'a str,
        _history: &'a [super::history::RuntimeMessage],
        _finishing: bool,
    ) -> AgentHostFuture<'a, Result<Vec<super::history::RuntimeMessage>, String>> {
        Box::pin(async { Ok(Vec::new()) })
    }
    fn workflow_hooks(&self) -> Option<&crate::chat::workflow_hooks::Runtime> {
        None
    }

    /// Consecutive automatic compaction failures remembered for the conversation, so the
    /// ZCode circuit breaker outlives one run. Default: nothing is remembered between runs.
    fn auto_compact_failures(&self, _conversation_id: &str) -> u32 {
        0
    }

    fn set_auto_compact_failures(&self, _conversation_id: &str, _failures: u32) {}

    /// Persist a best-effort snapshot of the in-progress assistant message to
    /// durable storage after a completed tool round. The full assistant message
    /// is otherwise written only once, after the loop returns; if the process
    /// dies mid-run (crash / forced exit) that whole turn — including tool work
    /// already done — is lost. This checkpoint keeps it recoverable on the next
    /// load. `api_messages` carries the loop's accumulated provider messages
    /// (assistant tool_calls + tool results) up to this round so the draft is
    /// replayable on a later "continue" — without them an `interrupted` draft
    /// loses all tool context and the model restarts from scratch. Default
    /// no-op for hosts that do not persist assistant drafts.
    fn persist_partial_assistant<'a>(
        &'a self,
        _conversation_id: &'a str,
        _message_id: &'a str,
        _tool_records: &'a [AgentToolCall],
        _segments: &'a [ContentBlock],
        _api_messages: &'a [super::history::RuntimeMessage],
    ) -> AgentHostFuture<'a, ()> {
        Box::pin(async {})
    }

    fn request_tool_approval<'a>(
        &'a self,
        ctx: &'a ToolExecutionContext<'a>,
        record: &'a AgentToolCall,
    ) -> AgentHostFuture<'a, bool>;

    /// Ask the user once per conversation to authorize the file/shell tool
    /// family (full-disk read/write + command execution). Hosts that can prompt
    /// (the chat window) surface a consent dialog and cache the grant.
    /// Default denies — a host must opt in.
    fn request_session_consent<'a>(
        &'a self,
        _ctx: &'a ToolExecutionContext<'a>,
    ) -> AgentHostFuture<'a, bool> {
        Box::pin(async { false })
    }

    fn request_user_response<'a>(
        &'a self,
        ctx: &'a ToolExecutionContext<'a>,
        record: &'a AgentToolCall,
        prompt: AskUserPromptPayload,
    ) -> AgentHostFuture<'a, AskUserResponseResult>;

    /// User-configured lifecycle hooks for this run, or `None` when the host has
    /// none (sub-agents, probe, tests) or the user configured zero enabled hooks.
    /// `None` is the whole zero-cost path: the loop never builds a payload.
    fn hooks(&self) -> Option<&HookDispatcher> {
        None
    }
}
