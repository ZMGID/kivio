//! Narrow model-execution port for the agent loop. The application adapter
//! owns provider credentials, failover and HTTP pools; a run only asks for the
//! three model operations it can perform.

use super::content::{ContentBlock};
use std::{future::Future, pin::Pin};

use serde_json::Value;

use crate::{
    chat::{
        model::{GenerateOptions, GenerateOutput, ModelError, RequestMetadata, ToolChoice},

    },
    mcp::ChatToolDefinition,
    settings::ModelProvider,
    state::AppState,
};

use super::{
    compaction::{summarize_history, CompactOutcome},
    planning::{generate_via_stream_collect, stream_scoped_chat_completion_inner},
    stream::{ToolCallDraftTracker, WebSearchCardTracker},
    types::AgentStreamPolicy,
};

pub(crate) type ProviderFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Clone)]
pub(crate) struct StreamRequest<'a> {
    pub host: &'a super::context::AgentContext<'a>,
    pub provider: &'a ModelProvider,
    pub model: &'a str,
    pub messages: Vec<super::history::RuntimeMessage>,
    pub tools: Option<&'a [ChatToolDefinition]>,
    pub tool_choice: ToolChoice,
    pub retry_attempts: usize,
    pub thinking_enabled: bool,
    pub thinking_level: Option<String>,
    pub builtin_web_search: bool,
    pub max_output_tokens: u32,
    pub conversation_id: &'a str,
    pub message_id: &'a str,
    pub label: &'a str,
    pub policy: AgentStreamPolicy,
    pub text_segment: Option<ContentBlock>,
    pub reasoning_segment: Option<ContentBlock>,
    pub tool_draft_tracker: Option<ToolCallDraftTracker>,
    pub web_search_tracker: Option<WebSearchCardTracker>,
}

pub(crate) struct MessageRequest<'a> {
    pub provider: &'a ModelProvider,
    pub model: &'a str,
    pub messages: Vec<super::history::RuntimeMessage>,
    pub tool_choice: ToolChoice,
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
    pub automatic: bool,
    /// The run's tools; the summary request carries them so tool history stays valid.
    pub tools: &'a [ChatToolDefinition],
    pub max_output_tokens: u32,
    pub conversation_id: &'a str,
    pub message_id: &'a str,
    pub cancel: Option<ProviderFuture<'a, ()>>,
    pub host: Option<&'a super::context::AgentContext<'a>>,
}

pub(crate) trait ProviderRuntime: Send + Sync {
    fn stream<'a>(
        &'a self,
        request: StreamRequest<'a>,
    ) -> ProviderFuture<'a, Result<GenerateOutput, ModelError>>;

    fn message<'a>(
        &'a self,
        request: MessageRequest<'a>,
    ) -> ProviderFuture<'a, Result<GenerateOutput, ModelError>>;

    fn summarize<'a>(&'a self, request: SummaryRequest<'a>) -> ProviderFuture<'a, CompactOutcome>;
}

fn allows_task_speculation(choice: ToolChoice, tools: Option<&[ChatToolDefinition]>) -> bool {
    choice != ToolChoice::None && tools.is_some_and(|tools| tools.iter().any(|tool| {
        tool.source == "native" && tool.name == crate::chat::sub_agent::TASK_TOOL_NAME
    }))
}

impl ProviderRuntime for AppState {
    fn stream<'a>(
        &'a self,
        request: StreamRequest<'a>,
    ) -> ProviderFuture<'a, Result<GenerateOutput, ModelError>> {
        if !allows_task_speculation(request.tool_choice, request.tools) {
            if let Some(tracker) = &request.tool_draft_tracker {
                tracker.disable_speculation();
            }
        }
        Box::pin(stream_scoped_chat_completion_inner(
            self,
            request.host,
            request.provider,
            request.model,
            request.messages,
            request.tools,
            request.tool_choice,
            request.retry_attempts,
            request.thinking_enabled,
            request.thinking_level,
            request.builtin_web_search,
            request.max_output_tokens,
            request.conversation_id,
            request.message_id,
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
    ) -> ProviderFuture<'a, Result<GenerateOutput, ModelError>> {
        let model_request = super::history::request_from_history(
            &request.messages,
            request.model,
            Vec::new(),
            GenerateOptions {
                thinking_enabled: request.thinking_enabled,
                thinking_level: request.thinking_level,
                builtin_web_search: request.builtin_web_search,
                tool_choice: request.tool_choice,
                max_tokens: request.max_output_tokens,
                ..GenerateOptions::default()
            },
            RequestMetadata {
                label: request.label.to_string(),
                conversation_id: Some(request.conversation_id.to_string()),
                message_id: Some(request.message_id.to_string()),
                ..Default::default()
            },
        );
        Box::pin(generate_via_stream_collect(
            self,
            request.provider,
            request.retry_attempts,
            model_request,
        ))
    }

    fn summarize<'a>(&'a self, request: SummaryRequest<'a>) -> ProviderFuture<'a, CompactOutcome> {
        Box::pin(summarize_history(
            self,
            request.provider,
            request.model,
            request.messages,
            request.automatic,
            request.tools,
            request.max_output_tokens,
            request.conversation_id,
            request.message_id,
            request.cancel,
            request.host,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speculation_requires_enabled_native_task_in_this_request() {
        let mut task = crate::chat::sub_agent::task::tool_definition(
            &[], &crate::settings::Settings::default(),
        );
        assert!(allows_task_speculation(ToolChoice::Auto, Some(std::slice::from_ref(&task))));
        assert!(!allows_task_speculation(ToolChoice::None, Some(std::slice::from_ref(&task))));
        assert!(!allows_task_speculation(ToolChoice::Auto, None));
        assert!(!allows_task_speculation(ToolChoice::Auto, Some(&[])));
        task.source = "mcp".into();
        assert!(!allows_task_speculation(ToolChoice::Auto, Some(std::slice::from_ref(&task))));
    }
}
