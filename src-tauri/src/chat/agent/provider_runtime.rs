//! Narrow model-execution port for the agent loop. The application adapter
//! owns provider credentials, failover and HTTP pools; a run only asks for the
//! three model operations it can perform.

use std::{future::Future, pin::Pin};

use serde_json::Value;

use crate::{
    chat::{
        model::{ModelError, ModelUsage},
        types::ChatMessageSegment,
    },
    mcp::ChatToolDefinition,
    settings::ModelProvider,
    state::AppState,
};

use super::{
    compaction::{summarize_history, CompactOutcome},
    host::AgentHost,
    planning::{call_chat_completion_message_with_usage, stream_scoped_chat_completion_inner},
    stream::{ChatStreamOutput, ToolCallDraftTracker, WebSearchCardTracker},
    types::AgentStreamPolicy,
};

pub(crate) type ProviderFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

pub(crate) struct StreamRequest<'a> {
    pub host: &'a dyn AgentHost,
    pub provider: &'a ModelProvider,
    pub model: &'a str,
    pub messages: Vec<Value>,
    pub tools: Option<&'a [ChatToolDefinition]>,
    pub retry_attempts: usize,
    pub thinking_enabled: bool,
    pub thinking_level: Option<String>,
    pub builtin_web_search: bool,
    pub max_output_tokens: u32,
    pub conversation_id: &'a str,
    pub run_id: &'a str,
    pub message_id: &'a str,
    pub generation: u64,
    pub label: &'a str,
    pub policy: AgentStreamPolicy,
    pub text_segment: Option<ChatMessageSegment>,
    pub reasoning_segment: Option<ChatMessageSegment>,
    pub tool_draft_tracker: Option<ToolCallDraftTracker>,
    pub web_search_tracker: Option<WebSearchCardTracker>,
}

pub(crate) struct MessageRequest<'a> {
    pub provider: &'a ModelProvider,
    pub model: &'a str,
    pub messages: Vec<Value>,
    pub retry_attempts: usize,
    pub thinking_enabled: bool,
    pub thinking_level: Option<String>,
    pub builtin_web_search: bool,
    pub max_output_tokens: u32,
    pub conversation_id: &'a str,
    pub message_id: &'a str,
    pub label: &'a str,
}

pub(crate) struct SummaryRequest<'a> {
    pub provider: &'a ModelProvider,
    pub model: &'a str,
    pub messages: &'a [Value],
    pub keep_tokens: usize,
    pub window: usize,
    pub max_output_tokens: u32,
    pub retry_attempts: usize,
    pub conversation_id: &'a str,
    pub message_id: &'a str,
    pub cancel: Option<ProviderFuture<'a, ()>>,
}

pub(crate) trait ProviderRuntime: Send + Sync {
    fn stream<'a>(
        &'a self,
        request: StreamRequest<'a>,
    ) -> ProviderFuture<'a, Result<ChatStreamOutput, ModelError>>;

    fn message<'a>(
        &'a self,
        request: MessageRequest<'a>,
    ) -> ProviderFuture<'a, Result<(Value, Option<ModelUsage>), String>>;

    fn summarize<'a>(&'a self, request: SummaryRequest<'a>) -> ProviderFuture<'a, CompactOutcome>;
}

impl ProviderRuntime for AppState {
    fn stream<'a>(
        &'a self,
        request: StreamRequest<'a>,
    ) -> ProviderFuture<'a, Result<ChatStreamOutput, ModelError>> {
        Box::pin(stream_scoped_chat_completion_inner(
            self,
            request.host,
            request.provider,
            request.model,
            request.messages,
            request.tools,
            request.retry_attempts,
            request.thinking_enabled,
            request.thinking_level,
            request.builtin_web_search,
            request.max_output_tokens,
            request.conversation_id,
            request.run_id,
            request.message_id,
            request.generation,
            request.label,
            request.policy,
            request.text_segment,
            request.reasoning_segment,
            request.tool_draft_tracker,
            request.web_search_tracker,
        ))
    }

    fn message<'a>(
        &'a self,
        request: MessageRequest<'a>,
    ) -> ProviderFuture<'a, Result<(Value, Option<ModelUsage>), String>> {
        Box::pin(call_chat_completion_message_with_usage(
            self,
            request.provider,
            request.model,
            request.messages,
            None,
            request.retry_attempts,
            request.thinking_enabled,
            request.thinking_level,
            request.builtin_web_search,
            request.max_output_tokens,
            request.conversation_id,
            request.message_id,
            request.label,
        ))
    }

    fn summarize<'a>(&'a self, request: SummaryRequest<'a>) -> ProviderFuture<'a, CompactOutcome> {
        Box::pin(summarize_history(
            self,
            request.provider,
            request.model,
            request.messages,
            request.keep_tokens,
            request.window,
            request.max_output_tokens,
            request.retry_attempts,
            request.conversation_id,
            request.message_id,
            None,
            request.cancel,
        ))
    }
}
