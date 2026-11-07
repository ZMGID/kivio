use crate::{settings::ModelProvider, state::AppState};

use crate::chat::agent::{
    events::RetryStatus,
    history::request_from_history,
    provider_runtime::{ModelCall, ModelDelivery, ModelOutputSink, ModelRuntime, ProviderFuture},
};

use super::{stream_with_chat_provider, GenerateOutput, ModelError};

pub(crate) struct AppAgentModel<'a> {
    state: &'a AppState,
    provider: &'a ModelProvider,
    retry_attempts: usize,
}

impl<'a> AppAgentModel<'a> {
    pub(crate) fn new(
        state: &'a AppState,
        provider: &'a ModelProvider,
        retry_attempts: usize,
    ) -> Self {
        Self {
            state,
            provider,
            retry_attempts,
        }
    }
}

impl ModelRuntime for AppAgentModel<'_> {
    fn generate<'a>(
        &'a self,
        call: ModelCall,
        sink: &'a mut dyn ModelOutputSink,
    ) -> ProviderFuture<'a, Result<GenerateOutput, ModelError>> {
        Box::pin(async move {
            sink.request_started(&call);
            let streamed = call.delivery == ModelDelivery::Streamed;
            let request = request_from_history(
                &call.history,
                &call.model,
                call.tools,
                call.options,
                call.metadata,
            );
            if !streamed {
                return stream_with_chat_provider(
                    self.state,
                    self.provider,
                    self.retry_attempts,
                    request,
                    sink,
                )
                .await;
            }
            crate::api::with_retry_budget(self.retry_attempts, async {
                let budget = crate::api::RetryBudget::current_or_new(self.retry_attempts);
                loop {
                    let result = {
                        let shared_sink = std::sync::Mutex::new(&mut *sink);
                        let mut stream_sink = |part| shared_sink.lock()
                            .unwrap_or_else(|error| error.into_inner()).emit(part);
                        let (progress_tx, mut progress_rx) = tokio::sync::mpsc::unbounded_channel();
                        let stream = crate::api::RETRY_PROGRESS.scope(
                            progress_tx,
                            stream_with_chat_provider(
                                self.state, self.provider, self.retry_attempts, request.clone(), &mut stream_sink,
                            ),
                        );
                        tokio::pin!(stream);
                        loop {
                            tokio::select! {
                                result = &mut stream => break result,
                                Some(retry) = progress_rx.recv() => shared_sink.lock()
                                    .unwrap_or_else(|error| error.into_inner()).retry(RetryStatus {
                                        reason: retry.reason, attempt: retry.attempt,
                                        max_retries: retry.max_retries, retry_at_ms: retry.retry_at_ms,
                                    }),
                            }
                        }
                    };
                    match result {
                        Err(error) if error.is_retryable_stream_error() => {
                            let Some((retry, delay_ms)) = budget.schedule(None, "Stream interrupted", false) else {
                                return Err(error);
                            };
                            sink.reset_attempt();
                            sink.retry(RetryStatus {
                                reason: retry.reason, attempt: retry.attempt,
                                max_retries: retry.max_retries, retry_at_ms: retry.retry_at_ms,
                            });
                            tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
                        }
                        result => return result,
                    }
                }
            }).await
        })
    }
}
