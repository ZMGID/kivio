use kivio_agent_sdk::{
    agent::{
        host::SessionInput,
        provider_runtime::{ModelCall, ModelOutputSink, ModelRuntime, ProviderFuture},
        AgentSession, AgentSessionConfig, InjectionAdmission, InjectionBatch, InjectionBoundary,
        InjectionContext, InjectionError, InjectionFuture, InjectionSource, InjectionSources,
        RuntimeMessage,
    },
    model::{GenerateOutput, ModelError, ModelRole},
};
use std::{
    collections::VecDeque,
    num::NonZeroU32,
    sync::{Arc, Mutex},
};

struct ReviewDraft;
impl InjectionSource for ReviewDraft {
    fn collect<'a>(
        &'a mut self,
        context: InjectionContext<'a>,
        admission: &'a mut InjectionAdmission,
    ) -> InjectionFuture<'a, Result<(), InjectionError>> {
        Box::pin(async move {
            if context.boundary != InjectionBoundary::BeforeFinish
                || !context
                    .run
                    .history
                    .last()
                    .is_some_and(|message| message.text_content() == "draft")
            {
                return Ok(());
            }
            admission.pending(InjectionBatch::new(vec![SessionInput::append(
                RuntimeMessage::text(ModelRole::User, "Review the draft before finishing."),
            )]));
            Ok(())
        })
    }
}

struct OfflineModel(Mutex<VecDeque<&'static str>>);
impl ModelRuntime for OfflineModel {
    fn generate<'a>(
        &'a self,
        call: ModelCall,
        sink: &'a mut dyn ModelOutputSink,
    ) -> ProviderFuture<'a, Result<GenerateOutput, ModelError>> {
        Box::pin(async move {
            sink.request_started(&call);
            let text = self
                .0
                .lock()
                .unwrap()
                .pop_front()
                .expect("unexpected request");
            if text == "reviewed" {
                assert_eq!(
                    call.history.last().unwrap().text_content(),
                    "Review the draft before finishing."
                );
            }
            Ok(GenerateOutput::text(text.into(), None))
        })
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let model = Arc::new(OfflineModel(Mutex::new(VecDeque::from([
        "draft", "reviewed",
    ]))));
    let mut config = AgentSessionConfig::new(model, "offline")
        .with_injections(InjectionSources::new().with(ReviewDraft));
    config.limits.max_model_steps = NonZeroU32::new(3);
    let session = AgentSession::new(config);
    let result = session.prompt("Prepare an answer.").await?;
    assert_eq!(result.content, "reviewed");
    assert_eq!(
        session
            .context()
            .iter()
            .filter(|message| message.text_content() == "draft")
            .count(),
        1
    );
    println!("{}", result.content);
    Ok(())
}
