use super::{control::RunStartError, types::AgentRunResult};
use crate::model::ModelError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentFailure {
    Model {
        stage: &'static str,
        source: ModelError,
    },
    Host {
        operation: &'static str,
        message: String,
    },
    Hook {
        hook: &'static str,
        message: String,
    },
    LimitExceeded {
        limit: RunLimit,
    },
    InvalidOutput(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunLimit {
    ModelSteps,
    Timeout,
}

impl AgentFailure {
    pub(crate) fn hook(hook: &'static str) -> impl FnOnce(String) -> Self {
        move |message| Self::Hook { hook, message }
    }

    pub(crate) fn host(operation: &'static str) -> impl FnOnce(String) -> Self {
        move |message| Self::Host { operation, message }
    }

    pub(crate) fn model(stage: &'static str, source: ModelError) -> Self {
        Self::Model { stage, source }
    }
}

impl std::fmt::Display for AgentFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Model { source, .. } => source.fmt(f),
            Self::Host { message, .. }
            | Self::Hook { message, .. }
            | Self::InvalidOutput(message) => f.write_str(message),
            Self::LimitExceeded { limit } => f.write_str(match limit {
                RunLimit::ModelSteps => "Agent model step limit exceeded",
                RunLimit::Timeout => "Agent run timeout exceeded",
            }),
        }
    }
}

impl std::error::Error for AgentFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Model { source, .. } => Some(source),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentAdmissionError {
    IdentityMismatch,
    Busy,
    EmptyInput,
    InvalidContinuation,
    InvalidHistory(String),
    NoRuntime,
    Control(RunStartError),
}

#[derive(Debug)]
pub enum AgentRunError {
    Admission(AgentAdmissionError),
    Failed {
        cause: AgentFailure,
        partial: Box<AgentRunResult>,
    },
}

impl std::fmt::Display for AgentRunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Admission(AgentAdmissionError::IdentityMismatch) => {
                f.write_str("Agent run identity does not match its control handle")
            }
            Self::Admission(AgentAdmissionError::Control(error)) => error.fmt(f),
            Self::Admission(AgentAdmissionError::Busy) => {
                f.write_str("Agent session is already running")
            }
            Self::Admission(AgentAdmissionError::EmptyInput) => {
                f.write_str("Agent prompt is empty")
            }
            Self::Admission(AgentAdmissionError::InvalidContinuation) => {
                f.write_str("Agent history cannot be continued")
            }
            Self::Admission(AgentAdmissionError::InvalidHistory(message)) => f.write_str(message),
            Self::Admission(AgentAdmissionError::NoRuntime) => {
                f.write_str("Agent session requires a Tokio runtime")
            }
            Self::Failed { cause, .. } => cause.fmt(f),
        }
    }
}

impl std::error::Error for AgentRunError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Failed { cause, .. } => Some(cause),
            Self::Admission(_) => None,
        }
    }
}
