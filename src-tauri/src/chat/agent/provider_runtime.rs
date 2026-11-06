use std::{future::Future, pin::Pin};

use crate::chat::model::{
    GenerateOptions, GenerateOutput, ModelError, ModelTool, RequestMetadata, StreamSink,
};

use super::{events::RetryStatus, history::RuntimeMessage};

pub type ProviderFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Clone)]
pub struct ModelCall {
    pub provider_id: String,
    pub model: String,
    pub history: Vec<RuntimeMessage>,
    pub tools: Vec<ModelTool>,
    pub options: GenerateOptions,
    pub metadata: RequestMetadata,
    pub delivery: ModelDelivery,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ModelDelivery {
    Streamed,
    Collected,
}

pub trait ModelRuntime: Send + Sync {
    /// Announce an effective request before streaming; synthetic completions make no announcement.
    fn generate<'a>(
        &'a self,
        call: ModelCall,
        sink: &'a mut dyn ModelOutputSink,
    ) -> ProviderFuture<'a, Result<GenerateOutput, ModelError>>;
}

pub trait ModelOutputSink: StreamSink {
    /// Receives the effective request before any output or speculative tool work.
    fn request_started(&mut self, call: &ModelCall);
    fn retry(&mut self, status: RetryStatus);
    fn reset_attempt(&mut self);
    fn draft_snapshot(&self) -> Vec<(String, String)>;
}

pub(crate) struct DiscardModelSink;

impl StreamSink for DiscardModelSink {
    fn emit(&mut self, _: crate::chat::model::StreamPart) -> Result<(), ModelError> {
        Ok(())
    }
}

impl ModelOutputSink for DiscardModelSink {
    fn request_started(&mut self, _: &ModelCall) {}
    fn retry(&mut self, _: RetryStatus) {}
    fn reset_attempt(&mut self) {}
    fn draft_snapshot(&self) -> Vec<(String, String)> {
        Vec::new()
    }
}
