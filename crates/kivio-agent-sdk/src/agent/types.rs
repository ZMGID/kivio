use super::content::{AgentToolCall, ContentBlock};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentRunEntry {
    Send,
    Regenerate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentPhase {
    ToolLoop,
    Synthesis,
    Plain,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentStreamPolicy {
    PlanningNoDoneUntilNoTools,
    SynthesisAlwaysDone,
    SynthesisDeferEmpty,
}

/// Optional bounds for a run. Provider transport retries are not model steps.
#[derive(Debug, Clone, Copy, Default)]
pub struct RunLimits {
    pub max_model_steps: Option<std::num::NonZeroU32>,
    pub timeout: Option<std::time::Duration>,
}

#[derive(Clone)]
pub struct AgentRunConfig<'a> {
    pub provider_runtime: &'a dyn super::provider_runtime::ModelRuntime,
    pub conversation_id: String,
    pub run_id: String,
    pub message_id: String,
    pub generation: u64,
    pub provider_id: String,
    pub provider_label: String,
    pub api_format: String,
    pub model: String,
    pub history: super::SessionHistory,
    pub tools: Vec<crate::tools::ToolDefinition>,
    pub blocked_tool_calls: Vec<crate::tools::ToolDefinition>,
    pub options: crate::model::GenerateOptions,
    pub capabilities: crate::model::ModelCapabilities,
    pub initial_cache_usage: Option<(u64, u64)>,
    pub report_context: bool,
    pub limits: RunLimits,
}

impl AgentRunConfig<'_> {
    pub(crate) fn validate_call(
        &self,
        call: &super::provider_runtime::ModelCall,
    ) -> Result<(), crate::model::ModelError> {
        use crate::model::{MessagePart, ModelError, ModelErrorKind, ToolChoice};
        let unknown = crate::model::ModelCapabilities::default();
        let capabilities = call.capabilities.as_ref().unwrap_or_else(|| {
            if call.provider_id == self.provider_id && call.model == self.model {
                &self.capabilities
            } else {
                &unknown
            }
        });
        let tools_enabled = call.options.tool_choice != ToolChoice::None;
        let native_tools = call
            .options
            .provider_options
            .get("tools")
            .and_then(serde_json::Value::as_array);
        let native_functions = native_tools.is_some_and(|tools| {
            tools.iter().any(|tool| {
                tool.get("type").and_then(serde_json::Value::as_str) == Some("function")
            })
        });
        if tools_enabled
            && capabilities.tools == Some(false)
            && (!call.tools.is_empty() || native_functions)
        {
            return Err(ModelError::with_kind(
                "Selected model does not support tools",
                ModelErrorKind::ToolsUnsupported,
            ));
        }
        let reasoning = call
            .options
            .thinking_level
            .as_deref()
            .is_some_and(|level| !matches!(level, "none" | "off"));
        if capabilities.reasoning == Some(false) && reasoning {
            return Err(ModelError::with_kind(
                "Selected model does not support reasoning",
                ModelErrorKind::InvalidRequest,
            ));
        }
        let native_search = native_tools.is_some_and(|tools| {
            tools.iter().any(|tool| {
                matches!(
                    tool.get("type").and_then(serde_json::Value::as_str),
                    Some("web_search" | "web_search_preview")
                )
            })
        });
        if tools_enabled
            && capabilities.builtin_web_search == Some(false)
            && (call.options.builtin_web_search || native_search)
        {
            return Err(ModelError::with_kind(
                "Selected model does not support built-in web search",
                ModelErrorKind::InvalidRequest,
            ));
        }
        if capabilities.vision == Some(false)
            && call
                .history
                .iter()
                .filter_map(|entry| entry.model_message())
                .any(|message| {
                    message.content.iter().any(|part| {
                        matches!(part, MessagePart::Image { data, .. } if !data.is_empty())
                            || matches!(part, MessagePart::ImageUrl { .. })
                    })
                })
        {
            return Err(ModelError::with_kind(
                "Selected model does not support image input",
                ModelErrorKind::InvalidRequest,
            ));
        }
        Ok(())
    }

    pub(crate) fn builtin_web_search_active(&self) -> bool {
        self.options.builtin_web_search
    }

    pub(crate) fn model_call(
        &self,
        history: Vec<super::history::RuntimeMessage>,
        tools: Vec<crate::model::ModelTool>,
        tool_choice: crate::model::ToolChoice,
        label: &str,
        delivery: super::provider_runtime::ModelDelivery,
    ) -> super::provider_runtime::ModelCall {
        super::provider_runtime::ModelCall {
            provider_id: self.provider_id.clone(),
            model: self.model.clone(),
            capabilities: None,
            history,
            tools,
            delivery,
            options: crate::model::GenerateOptions {
                tool_choice: if tool_choice == crate::model::ToolChoice::None {
                    crate::model::ToolChoice::None
                } else {
                    self.options.tool_choice
                },
                ..self.options.clone()
            },
            metadata: crate::model::RequestMetadata {
                label: label.into(),
                conversation_id: Some(self.conversation_id.clone()),
                message_id: Some(self.message_id.clone()),
                ..Default::default()
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentRunStatus {
    Completed,
    Recovered,
    CompactionBlocked,
    Cancelled,
    Failed(super::error::AgentFailure),
}

#[derive(Debug, Clone)]
pub struct AgentRunResult {
    pub(crate) final_message: Option<crate::model::ModelMessage>,
    pub content: String,
    pub reasoning: Option<String>,
    pub tool_records: Vec<AgentToolCall>,
    pub segments: Vec<ContentBlock>,
    pub generated_history: Vec<super::history::RuntimeMessage>,
    pub status: AgentRunStatus,
    /// Accumulated reported model usage. Absent when no request reports usage.
    pub usage: Option<crate::model::ModelUsage>,
    /// Sum of complete per-request cache telemetry pairs; independent of billing usage.
    pub cache_pairs: Option<(u64, u64)>,
    /// Usage from the latest model request, separate from accumulated billing usage.
    pub last_step_usage: Option<crate::model::ModelUsage>,
    pub history: super::SessionHistory,
    pub compacted: bool,
    /// Generated image payloads accumulated during this run. The host owns persistence.
    pub images: Vec<crate::model::GeneratedImageData>,
    /// Inputs taken from the host but not consumed, in original queue order.
    pub pending_steering: Vec<super::steering::SteeringMessage>,
    pub pending_follow_up: Vec<super::steering::SteeringMessage>,
}
