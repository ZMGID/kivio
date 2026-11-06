use std::ops::Deref;

use super::{
    content::{AgentToolCall, ContentBlock},
    control::AgentRunHandle,
    events::{AgentEventKind, AgentEvents},
    host::AgentHost,
};

pub(crate) struct AgentContext<'a> {
    pub(crate) host: &'a dyn AgentHost,
    pub(crate) events: AgentEvents<'a>,
    pub(crate) control: AgentRunHandle<'a>,
}

impl<'a> Deref for AgentContext<'a> {
    type Target = dyn AgentHost + 'a;
    fn deref(&self) -> &Self::Target {
        self.host
    }
}

impl AgentContext<'_> {
    pub(crate) fn content(
        &self,
        delta: &str,
        reasoning_delta: Option<&str>,
        block: Option<&ContentBlock>,
    ) {
        self.events.emit(AgentEventKind::Content {
            delta,
            reasoning_delta,
            block,
        });
    }
    pub(crate) fn tool_updated(&self, call: &AgentToolCall) {
        self.events.emit(AgentEventKind::ToolUpdated(call));
    }
    pub(crate) fn search_updated(&self, block: &ContentBlock, announce: bool) {
        self.events
            .emit(AgentEventKind::SearchUpdated { block, announce });
    }
    pub(crate) fn discard_attempt(
        &self,
        text_chars: u32,
        reasoning_chars: u32,
        block_ids: Vec<u32>,
        tool_ids: Vec<String>,
    ) {
        self.events.emit(AgentEventKind::AttemptDiscarded {
            text_chars,
            reasoning_chars,
            block_ids,
            tool_ids,
        });
    }
    pub(crate) fn status(&self, note: Option<String>) {
        self.events.emit(AgentEventKind::Status { note });
    }
    pub(crate) fn retry(&self, retry: super::events::RetryStatus) {
        self.events.emit(AgentEventKind::Retry(retry));
    }
    pub(crate) fn compaction(
        &self,
        phase: &str,
        trigger: Option<&str>,
        boundary: Option<&crate::chat::types::CompactionBoundaryRecord>,
    ) {
        self.events.emit(AgentEventKind::Compaction {
            phase,
            trigger,
            boundary: boundary.map(|boundary| super::events::CompactionFact {
                id: &boundary.id,
                covered_through: &boundary.source_until_message_id,
                history_tail: boundary.display_after_message_id.as_deref(),
                tokens_before: boundary.token_estimate_before,
                tokens_after: boundary.token_estimate_after,
                summary: &boundary.summary_content,
                trigger: &boundary.trigger,
                created_at: boundary.created_at,
            }),
        });
    }
    pub(crate) fn context_prepared(
        &self,
        provider_id: &str,
        model: &str,
        messages: &[super::history::RuntimeMessage],
        tools: &[crate::mcp::ChatToolDefinition],
    ) {
        self.events.emit(AgentEventKind::ContextPrepared {
            provider_id,
            model,
            messages,
            tools,
        });
    }
    pub(crate) fn context_cache(&self, cache: Option<(u64, u64)>) {
        self.events.emit(AgentEventKind::ContextCache { cache });
    }
    pub(crate) fn context_unreported(&self) {
        self.events.emit(AgentEventKind::ContextUnreported);
    }
    pub(crate) fn context_usage(
        &self,
        used_tokens: u64,
        source: Option<&str>,
        window: Option<u64>,
        cache: Option<(u64, u64)>,
    ) {
        self.events.emit(AgentEventKind::ContextUsage {
            used_tokens,
            source,
            window,
            cache,
        });
    }
}
