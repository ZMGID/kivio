use std::panic::{catch_unwind, AssertUnwindSafe};

use super::{events::AgentRunIdentity, host::SessionInput, session::RunSnapshot};
use crate::model::{MessagePart, ModelRole};

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
    /// At BeforeFinish, history includes the candidate assistant answer.
    pub run: RunSnapshot<'a>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InjectionSettlement {
    /// Appended to runtime history, not necessarily persisted or read by a model.
    Applied,
    /// Not appended. The source may make its reserved inputs available again.
    Released,
}

/// Owns a reservation until the loop applies it or drops it.
#[derive(Default)]
pub struct InjectionBatch {
    inputs: Vec<SessionInput>,
    settlement: Option<Box<dyn FnOnce(InjectionSettlement) + Send>>,
}

impl InjectionBatch {
    pub fn new(inputs: Vec<SessionInput>) -> Self {
        Self {
            inputs,
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

    pub(crate) fn take_inputs(&mut self) -> Vec<SessionInput> {
        std::mem::take(&mut self.inputs)
    }

    pub(crate) fn applied(mut self) {
        self.settle(InjectionSettlement::Applied);
    }

    fn settle(&mut self, outcome: InjectionSettlement) {
        if let Some(callback) = self.settlement.take() {
            let _ = catch_unwind(AssertUnwindSafe(|| callback(outcome)));
        }
    }
}

impl Drop for InjectionBatch {
    fn drop(&mut self) {
        self.settle(InjectionSettlement::Released);
    }
}

pub trait InjectionSource: Send {
    /// Synchronous and nonblocking. Fetch asynchronous inputs into a host-owned queue first.
    fn collect(&mut self, context: InjectionContext<'_>) -> Result<InjectionBatch, String>;
}

/// Ordered sources, without an SDK mailbox. Limits cover all sources at one boundary.
pub struct InjectionSources {
    sources: Vec<Box<dyn InjectionSource>>,
    max_messages: usize,
    max_bytes: usize,
}

impl Default for InjectionSources {
    fn default() -> Self {
        Self {
            sources: Vec::new(),
            max_messages: 64,
            max_bytes: 1024 * 1024,
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

    pub(crate) fn collect(
        &mut self,
        context: InjectionContext<'_>,
    ) -> Result<Vec<InjectionBatch>, String> {
        let mut batches = Vec::new();
        let mut messages = 0usize;
        let mut bytes = 0usize;
        for source in &mut self.sources {
            let batch = catch_unwind(AssertUnwindSafe(|| source.collect(context)))
                .map_err(|_| "Injection source panicked".to_owned())??;
            for input in &batch.inputs {
                let message = input
                    .message
                    .model_message()
                    .ok_or("Injection requires a user or developer message")?;
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
                    return Err("Injection requires user or developer input without assistant or tool blocks".into());
                }
                messages = messages.saturating_add(1);
                bytes = bytes.saturating_add(
                    serde_json::to_vec(&input.message)
                        .map_err(|error| error.to_string())?
                        .len(),
                );
                if messages > self.max_messages || bytes > self.max_bytes {
                    return Err(
                        "Injection boundary exceeds configured message or byte limit".into(),
                    );
                }
            }
            if !batch.inputs.is_empty() {
                batches.push(batch);
            }
        }
        Ok(batches)
    }
}
