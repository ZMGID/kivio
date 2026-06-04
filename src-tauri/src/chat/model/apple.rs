use std::sync::Arc;

use crate::apple_intelligence::AppleIntelligenceClient;

use super::{
    GenerateOutput, GenerateRequest, LanguageModelProvider, ModelError, ModelFuture,
    ProviderCapabilities, StreamPart, StreamSink,
};

pub struct AppleLocalProvider {
    client: Arc<AppleIntelligenceClient>,
}

impl AppleLocalProvider {
    pub fn new(client: Arc<AppleIntelligenceClient>) -> Self {
        Self { client }
    }
}

impl LanguageModelProvider for AppleLocalProvider {
    fn generate<'a>(&'a self, request: GenerateRequest) -> ModelFuture<'a, GenerateOutput> {
        Box::pin(async move {
            if !request.tools.is_empty() {
                return Err(ModelError::new(
                    "Apple local provider does not support tools",
                ));
            }
            let prompt = apple_prompt_from_request(&request);
            let text = self
                .client
                .call_text(&prompt)
                .await
                .map_err(ModelError::new)?;
            Ok(GenerateOutput::text(
                text.clone(),
                None,
                serde_json::json!({
                    "role": "assistant",
                    "content": text,
                }),
            ))
        })
    }

    fn stream<'a>(
        &'a self,
        request: GenerateRequest,
        sink: &'a mut (dyn StreamSink + Send),
    ) -> ModelFuture<'a, GenerateOutput> {
        Box::pin(async move {
            if !request.tools.is_empty() {
                return Err(ModelError::new(
                    "Apple local provider does not support tools",
                ));
            }
            let prompt = apple_prompt_from_request(&request);
            let mut full = String::new();
            self.client
                .stream_text(&prompt, |delta| {
                    full.push_str(delta);
                    let _ = sink.emit(StreamPart::TextDelta {
                        delta: delta.to_string(),
                    });
                })
                .await
                .map_err(ModelError::new)?;
            sink.emit(StreamPart::Finish {
                reason: "stop".to_string(),
                full: full.clone(),
            })?;
            Ok(GenerateOutput::text(
                full.clone(),
                None,
                serde_json::json!({
                    "role": "assistant",
                    "content": full,
                }),
            ))
        })
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            tool_calling: false,
            vision: false,
            streaming: true,
            reasoning: false,
        }
    }
}

fn apple_prompt_from_request(request: &GenerateRequest) -> String {
    let mut parts = Vec::new();
    if !request.system.trim().is_empty() {
        parts.push(format!("System:\n{}", request.system.trim()));
    }
    for message in &request.messages {
        let role = match message.role {
            super::ModelRole::Assistant => "Assistant",
            _ => "User",
        };
        let content = message.text_content();
        if !content.trim().is_empty() {
            parts.push(format!("{role}:\n{}", content.trim()));
        }
    }
    parts.push("Assistant:".to_string());
    parts.join("\n\n")
}
