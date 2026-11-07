use std::{future::Future, pin::Pin};

use crate::model::{
    GenerateOptions, GenerateOutput, ModelError, ModelTool, RequestMetadata, StreamSink,
};

use super::{events::RetryStatus, history::RuntimeMessage};

pub type ProviderFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Clone)]
pub struct ModelCall {
    pub provider_id: String,
    pub model: String,
    /// Optional capability snapshot for this request. A changed model otherwise has unknown capabilities.
    pub capabilities: Option<crate::model::ModelCapabilities>,
    pub history: Vec<RuntimeMessage>,
    pub tools: Vec<ModelTool>,
    pub options: GenerateOptions,
    pub metadata: RequestMetadata,
    pub delivery: ModelDelivery,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ModelDelivery {
    Streamed,
    /// Return aggregate output without visible content frames; transport may still use SSE.
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

pub(crate) async fn generate_with_provider(
    provider: &(dyn crate::model::LanguageModelProvider + Sync),
    call: ModelCall,
    sink: &mut dyn ModelOutputSink,
) -> Result<GenerateOutput, ModelError> {
    sink.request_started(&call);
    let request = super::history::request_from_history(
        &call.history,
        &call.model,
        call.tools,
        call.options,
        call.metadata,
    );
    if call.delivery == ModelDelivery::Collected {
        provider.stream(request, &mut |_part| Ok(())).await
    } else {
        provider.stream(request, sink).await
    }
}

pub struct DiscardModelSink;

impl StreamSink for DiscardModelSink {
    fn emit(&mut self, _: crate::model::StreamPart) -> Result<(), ModelError> {
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
