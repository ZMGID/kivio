use std::{
    future::Future,
    panic::{catch_unwind, AssertUnwindSafe},
    pin::Pin,
};

use super::{
    error::AgentFailure, events::AgentRunIdentity, host::SessionInput, session::RunSnapshot,
    SessionHistory,
};
use crate::model::{MessagePart, ModelRole};

pub type InjectionFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InjectionBoundary {
    BeforeStep,
    BeforeFinish,
}

#[derive(Clone, Copy)]
pub struct InjectionContext<'a> {
    pub boundary: InjectionBoundary,
    pub identity: AgentRunIdentity<'a>,
    pub invocation_id: u64,
    /// At BeforeFinish, both snapshots include the candidate assistant answer.
    pub run: RunSnapshot<'a>,
    pub history: &'a SessionHistory,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InjectionError {
    InvalidInput(String),
    Source(String),
    Panicked,
    LimitExceeded,
    Persistence(String),
    Unregistered,
}
impl std::fmt::Display for InjectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidInput(message) | Self::Source(message) | Self::Persistence(message) => {
                f.write_str(message)
            }
            Self::Panicked => f.write_str("Injection source or builder panicked"),
            Self::LimitExceeded => {
                f.write_str("Injection boundary exceeds configured message or byte limit")
            }
            Self::Unregistered => f.write_str("Injection kind is unregistered"),
        }
    }
}
impl std::error::Error for InjectionError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropReason {
    Stale,
    Cleared,
    Unregistered,
    RunStopped,
    Released,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InjectionOutcome {
    /// Appended to live history, not a promise of persistence or a model request.
    Applied,
    Dropped(DropReason),
    Failed(InjectionError),
}

/// Validated inputs. Construct before saving a producer-owned durable transaction.
pub struct PreparedInjection(Vec<SessionInput>);
impl PreparedInjection {
    pub fn new(inputs: Vec<SessionInput>) -> Result<Self, InjectionError> {
        validate_inputs(&inputs)?;
        Ok(Self(inputs))
    }
}

pub(crate) fn validate_inputs(inputs: &[SessionInput]) -> Result<usize, InjectionError> {
    let mut bytes = 0usize;
    for input in inputs {
        let message = input.message.model_message().ok_or_else(|| {
            InjectionError::InvalidInput("Injection requires a user or developer message".into())
        })?;
        if !matches!(message.role, ModelRole::User | ModelRole::Developer)
            || message.content.iter().any(|part| {
                !matches!(
                    part,
                    MessagePart::Text { .. }
                        | MessagePart::Image { .. }
                        | MessagePart::ImageUrl { .. }
                        | MessagePart::Video { .. }
                )
            })
        {
            return Err(InjectionError::InvalidInput(
                "Injection requires user or developer input without assistant or tool blocks"
                    .into(),
            ));
        }
        bytes = bytes.saturating_add(
            serde_json::to_vec(&input.message)
                .map_err(|error| InjectionError::InvalidInput(error.to_string()))?
                .len(),
        );
    }
    Ok(bytes)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InjectionSettlement {
    Applied,
    Released,
}

pub(crate) trait PendingInjection: Send {
    fn build(&mut self, context: InjectionContext<'_>)
        -> Result<Vec<SessionInput>, InjectionError>;
    fn apply(
        &mut self,
        inputs: Vec<SessionInput>,
        append: &mut dyn FnMut(Vec<SessionInput>) -> Result<(), AgentFailure>,
    ) -> Result<bool, AgentFailure>;
    fn settle(&mut self, outcome: InjectionOutcome);
}

#[derive(Default)]
pub struct InjectionBatch {
    inputs: Vec<SessionInput>,
    pub(crate) lazy: Option<Box<dyn PendingInjection>>,
    settlement: Option<Box<dyn FnOnce(InjectionSettlement) + Send>>,
}
impl InjectionBatch {
    pub fn new(inputs: Vec<SessionInput>) -> Self {
        Self {
            inputs,
            lazy: None,
            settlement: None,
        }
    }
    pub fn with_settlement(
        mut self,
        settlement: impl FnOnce(InjectionSettlement) + Send + 'static,
    ) -> Self {
        self.settlement = Some(Box::new(settlement));
        self
    }
    pub(crate) fn build(
        &mut self,
        context: InjectionContext<'_>,
    ) -> Result<Vec<SessionInput>, InjectionError> {
        catch_unwind(AssertUnwindSafe(|| match &mut self.lazy {
            Some(lazy) => lazy.build(context),
            None => Ok(std::mem::take(&mut self.inputs)),
        }))
        .unwrap_or(Err(InjectionError::Panicked))
    }
    pub(crate) fn apply(
        mut self,
        inputs: Vec<SessionInput>,
        append: &mut dyn FnMut(Vec<SessionInput>) -> Result<(), AgentFailure>,
    ) -> Result<bool, AgentFailure> {
        let applied = if let Some(lazy) = &mut self.lazy {
            lazy.apply(inputs, append)?
        } else if inputs.is_empty() {
            false
        } else {
            append(inputs)?;
            true
        };
        self.settle(if applied {
            InjectionOutcome::Applied
        } else {
            InjectionOutcome::Dropped(DropReason::Released)
        });
        Ok(applied)
    }
    pub(crate) fn settle(&mut self, outcome: InjectionOutcome) {
        if let Some(mut lazy) = self.lazy.take() {
            lazy.settle(outcome.clone());
        }
        if let Some(callback) = self.settlement.take() {
            let settlement = if outcome == InjectionOutcome::Applied {
                InjectionSettlement::Applied
            } else {
                InjectionSettlement::Released
            };
            let _ = catch_unwind(AssertUnwindSafe(|| callback(settlement)));
        }
    }
}
impl Drop for InjectionBatch {
    fn drop(&mut self) {
        self.settle(InjectionOutcome::Dropped(DropReason::Released));
    }
}

/// Source-local facts survive a later error, panic, or stop during collection.
#[derive(Default)]
pub struct InjectionAdmission {
    pub(crate) committed: Vec<SessionInput>,
    pub(crate) pending: Vec<InjectionBatch>,
}
impl InjectionAdmission {
    pub fn committed(&mut self, prepared: PreparedInjection) {
        self.committed.extend(prepared.0);
    }
    pub fn pending(&mut self, batch: InjectionBatch) {
        self.pending.push(batch);
    }
    /// Facts committed during this call, not yet reflected in the borrowed context.
    pub fn committed_inputs(&self) -> &[SessionInput] {
        &self.committed
    }
}

pub trait InjectionSource: Send {
    /// Finish in-flight durable writes before returning. The SDK does not cancel this future.
    fn collect<'a>(
        &'a mut self,
        context: InjectionContext<'a>,
        admission: &'a mut InjectionAdmission,
    ) -> InjectionFuture<'a, Result<(), InjectionError>>;
}

pub struct InjectionSources {
    pub(crate) sources: Vec<Box<dyn InjectionSource>>,
    pub(crate) max_messages: usize,
    pub(crate) max_bytes: usize,
    pub(crate) errors: Vec<InjectionError>,
}
impl Default for InjectionSources {
    fn default() -> Self {
        Self {
            sources: Vec::new(),
            max_messages: 64,
            max_bytes: 1024 * 1024,
            errors: Vec::new(),
        }
    }
}
impl InjectionSources {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn with(mut self, source: impl InjectionSource + 'static) -> Self {
        self.push(source);
        self
    }
    pub fn push(&mut self, source: impl InjectionSource + 'static) {
        self.sources.push(Box::new(source));
    }
    pub fn with_limits(mut self, max_messages: usize, max_bytes: usize) -> Self {
        self.max_messages = max_messages;
        self.max_bytes = max_bytes;
        self
    }
    /// Nonfatal collection/build errors from the most recent run.
    pub fn errors(&self) -> &[InjectionError] {
        &self.errors
    }
}
