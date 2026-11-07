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
    InvalidOutput(String),
}

impl AgentFailure {
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
            Self::Host { message, .. } | Self::InvalidOutput(message) => f.write_str(message),
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
