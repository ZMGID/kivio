use kivio_agent_sdk::{
    agent::{
        host::{InputPersistence, SessionInput},
        provider_runtime::{ModelCall, ModelOutputSink, ModelRuntime, ProviderFuture},
        run_agent_loop,
        session::NoSessionPolicy,
        InjectionBatch, InjectionBoundary, InjectionContext, InjectionSettlement, InjectionSource,
        InjectionSources, RuntimeMessage,
    },
    model::{GenerateOutput, ModelError, ModelRole, StreamPart},
};
use kivio_agent_sdk_consumer::{config, handle, initial_history, Control, EchoTool, Events, Host};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

struct Model {
    outputs: Mutex<VecDeque<GenerateOutput>>,
    calls: Mutex<Vec<ModelCall>>,
}
impl Model {
    fn new(outputs: Vec<GenerateOutput>) -> Self {
        Self {
            outputs: Mutex::new(outputs.into()),
            calls: Mutex::new(vec![]),
        }
    }
}
impl ModelRuntime for Model {
    fn generate<'a>(
        &'a self,
        call: ModelCall,
        sink: &'a mut dyn ModelOutputSink,
    ) -> ProviderFuture<'a, Result<GenerateOutput, ModelError>> {
        Box::pin(async move {
            sink.request_started(&call);
            self.calls.lock().unwrap().push(call);
            let output = self
                .outputs
                .lock()
                .unwrap()
                .pop_front()
                .expect("unexpected model call");
            if let Some(reasoning) = &output.reasoning {
                sink.emit(StreamPart::ReasoningDelta {
                    delta: reasoning.clone(),
                })?;
            }
            sink.emit(StreamPart::TextDelta {
                delta: output.text.clone(),
            })?;
            Ok(output)
        })
    }
}

fn output(text: &str) -> GenerateOutput {
    GenerateOutput::text(format!("{text}\n"), None)
}

struct Source(Box<dyn FnMut(InjectionContext<'_>) -> Result<InjectionBatch, String> + Send>);
impl InjectionSource for Source {
    fn collect(&mut self, context: InjectionContext<'_>) -> Result<InjectionBatch, String> {
        (self.0)(context)
    }
}
fn source(
    f: impl FnMut(InjectionContext<'_>) -> Result<InjectionBatch, String> + Send + 'static,
) -> Source {
    Source(Box::new(f))
}
fn input(text: &str) -> SessionInput {
    SessionInput::append(RuntimeMessage::text(ModelRole::User, text))
}
fn reserved(text: &str, outcomes: &Arc<Mutex<Vec<InjectionSettlement>>>) -> InjectionBatch {
    let outcomes = outcomes.clone();
    InjectionBatch::new(vec![input(text)])
        .with_settlement(move |outcome| outcomes.lock().unwrap().push(outcome))
}

#[tokio::test]
async fn ordered_sources_apply_once_and_preserve_persistence_mode() {
    let model = Model::new(vec![output("done")]);
    let control = Arc::new(Control::default());
    let applied = Arc::new(Mutex::new(vec![]));
    let outcome = applied.clone();
    let mut injections = InjectionSources::new()
        .with(source(move |context| {
            if context.boundary == InjectionBoundary::BeforeStep {
                Ok(reserved("first", &outcome))
            } else {
                Ok(InjectionBatch::default())
            }
        }))
        .with(source(|context| {
            if context.boundary == InjectionBoundary::BeforeStep {
                Ok(InjectionBatch::new(vec![SessionInput {
                    message: input("saved").message,
                    persistence: InputPersistence::AlreadyPersisted,
                }]))
            } else {
                Ok(InjectionBatch::default())
            }
        }));
    let mut cfg = config(&model, initial_history());
    cfg.tools.clear();
    let result = run_agent_loop(
        cfg,
        &Host,
        Arc::new(EchoTool::new(true)),
        &mut NoSessionPolicy,
        &mut injections,
        &Events::default(),
        handle(&control),
    )
    .await
    .unwrap();
    let calls = model.calls.lock().unwrap();
    let texts: Vec<_> = calls[0]
        .history
        .iter()
        .map(RuntimeMessage::text_content)
        .collect();
    assert_eq!(&texts[texts.len() - 2..], &["first", "saved"]);
    assert_eq!(
        result
            .history
            .context()
            .iter()
            .filter(|m| m.text_content() == "saved")
            .count(),
        1
    );
    assert_eq!(
        result
            .generated_history
            .iter()
            .filter(|m| m.text_content() == "first")
            .count(),
        1
    );
    assert!(!result
        .generated_history
        .iter()
        .any(|m| m.text_content() == "saved"));
    assert_eq!(*applied.lock().unwrap(), vec![InjectionSettlement::Applied]);
}

#[tokio::test]
async fn empty_sources_do_not_continue_or_change_history() {
    let model = Model::new(vec![output("done")]);
    let control = Arc::new(Control::default());
    let seen = Arc::new(Mutex::new(vec![]));
    let log = seen.clone();
    let mut injections = InjectionSources::new().with(source(move |context| {
        log.lock().unwrap().push(context.boundary);
        Ok(InjectionBatch::default())
    }));
    let mut cfg = config(&model, initial_history());
    cfg.tools.clear();
    let result = run_agent_loop(
        cfg,
        &Host,
        Arc::new(EchoTool::new(true)),
        &mut NoSessionPolicy,
        &mut injections,
        &Events::default(),
        handle(&control),
    )
    .await
    .unwrap();
    assert_eq!(model.calls.lock().unwrap().len(), 1);
    assert_eq!(
        *seen.lock().unwrap(),
        vec![
            InjectionBoundary::BeforeStep,
            InjectionBoundary::BeforeFinish
        ]
    );
    assert_eq!(result.history.context().len(), initial_history().len() + 1);
}

#[tokio::test]
async fn cancellation_releases_reservations_before_application() {
    let model = Model::new(vec![]);
    let control = Arc::new(Control::default());
    let run = handle(&control);
    let stop = run.clone();
    let outcomes = Arc::new(Mutex::new(vec![]));
    let log = outcomes.clone();
    let mut injections = InjectionSources::new().with(source(move |_| {
        stop.stop();
        Ok(reserved("reserved", &log))
    }));
    let mut cfg = config(&model, initial_history());
    cfg.tools.clear();
    let result = run_agent_loop(
        cfg,
        &Host,
        Arc::new(EchoTool::new(true)),
        &mut NoSessionPolicy,
        &mut injections,
        &Events::default(),
        run,
    )
    .await
    .unwrap();
    assert_eq!(
        *outcomes.lock().unwrap(),
        vec![InjectionSettlement::Released]
    );
    assert!(!result
        .history
        .context()
        .iter()
        .any(|m| m.text_content() == "reserved"));
    assert!(model.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn failed_invalid_panicking_and_oversized_sources_release_every_reservation() {
    for mode in 0..5 {
        let model = Model::new(vec![]);
        let control = Arc::new(Control::default());
        let outcomes = Arc::new(Mutex::new(vec![]));
        let first = outcomes.clone();
        let second = outcomes.clone();
        let mut injections = InjectionSources::new()
            .with(source(move |_| Ok(reserved("first", &first))))
            .with(source(move |_| match mode {
                0 => Err("source failed".into()),
                1 => panic!("source panic"),
                2 => {
                    let mut invalid = input("invalid");
                    invalid.message = RuntimeMessage::system("bad");
                    let log = second.clone();
                    Ok(InjectionBatch::new(vec![invalid])
                        .with_settlement(move |o| log.lock().unwrap().push(o)))
                }
                _ => Ok(reserved("second", &second)),
            }))
            .with_limits(
                if mode == 3 { 1 } else { 64 },
                if mode == 4 { 1 } else { 1024 * 1024 },
            );
        let mut cfg = config(&model, initial_history());
        cfg.tools.clear();
        let error = run_agent_loop(
            cfg,
            &Host,
            Arc::new(EchoTool::new(true)),
            &mut NoSessionPolicy,
            &mut injections,
            &Events::default(),
            handle(&control),
        )
        .await
        .unwrap_err();
        assert!(matches!(
            error,
            kivio_agent_sdk::agent::AgentRunError::Failed {
                cause: kivio_agent_sdk::agent::AgentFailure::Hook {
                    hook: "injection.collect",
                    ..
                },
                ..
            }
        ));
        let settled = outcomes.lock().unwrap();
        assert_eq!(settled.len(), if mode < 2 || mode == 4 { 1 } else { 2 });
        assert!(settled.iter().all(|o| *o == InjectionSettlement::Released));
        assert!(model.calls.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn applied_callback_panic_does_not_release_or_lose_history() {
    let model = Model::new(vec![output("done")]);
    let control = Arc::new(Control::default());
    let outcomes = Arc::new(Mutex::new(vec![]));
    let log = outcomes.clone();
    let mut injections = InjectionSources::new().with(source(move |context| {
        if context.boundary != InjectionBoundary::BeforeStep {
            return Ok(InjectionBatch::default());
        }
        let log = log.clone();
        Ok(
            InjectionBatch::new(vec![input("kept")]).with_settlement(move |outcome| {
                log.lock().unwrap().push(outcome);
                panic!("observer panic");
            }),
        )
    }));
    let mut cfg = config(&model, initial_history());
    cfg.tools.clear();
    let result = run_agent_loop(
        cfg,
        &Host,
        Arc::new(EchoTool::new(true)),
        &mut NoSessionPolicy,
        &mut injections,
        &Events::default(),
        handle(&control),
    )
    .await
    .unwrap();
    assert_eq!(
        *outcomes.lock().unwrap(),
        vec![InjectionSettlement::Applied]
    );
    assert!(result
        .history
        .context()
        .iter()
        .any(|m| m.text_content() == "kept"));
}

#[tokio::test]
async fn owned_session_sources_reset_using_invocation_identity() {
    use kivio_agent_sdk::agent::{AgentSession, AgentSessionConfig};
    let model = Arc::new(Model::new(vec![output("first"), output("second")]));
    let seen = Arc::new(Mutex::new(vec![]));
    let log = seen.clone();
    let mut last = None;
    let injections = InjectionSources::new().with(source(move |context| {
        if last == Some(context.invocation_id) {
            return Ok(InjectionBatch::default());
        }
        last = Some(context.invocation_id);
        log.lock().unwrap().push(context.invocation_id);
        Ok(InjectionBatch::new(vec![input("per invocation")]))
    }));
    let session = AgentSession::new(
        AgentSessionConfig::new(model.clone(), "test-model").with_injections(injections),
    );
    session.prompt("first task").await.unwrap();
    session.prompt("second task").await.unwrap();
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 2);
    assert_ne!(seen[0], seen[1]);
    assert_eq!(
        session
            .context()
            .iter()
            .filter(|m| m.text_content() == "per invocation")
            .count(),
        2
    );
    assert_eq!(model.calls.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn pending_followups_precede_persistent_finish_sources() {
    let model = Model::new(vec![output("first"), output("second"), output("third")]);
    let control = Arc::new(Control::default());
    let run = handle(&control);
    run.follow_up(
        kivio_agent_sdk::agent::SteeringMessage::new("follow".into(), "user followup").unwrap(),
    );
    let seen = Arc::new(Mutex::new(vec![]));
    let log = seen.clone();
    let outcomes = Arc::new(Mutex::new(vec![]));
    let outcome = outcomes.clone();
    let mut injections = InjectionSources::new().with(source(move |context| {
        if context.boundary != InjectionBoundary::BeforeFinish {
            return Ok(InjectionBatch::default());
        }
        log.lock().unwrap().push(
            context
                .run
                .history
                .iter()
                .map(RuntimeMessage::text_content)
                .collect::<Vec<_>>(),
        );
        Ok(reserved("keep going", &outcome))
    }));
    let mut cfg = config(&model, initial_history());
    cfg.tools.clear();
    cfg.limits.max_model_steps = std::num::NonZeroU32::new(3);
    let error = run_agent_loop(
        cfg,
        &Host,
        Arc::new(EchoTool::new(true)),
        &mut NoSessionPolicy,
        &mut injections,
        &Events::default(),
        run,
    )
    .await
    .unwrap_err();
    assert!(matches!(
        error,
        kivio_agent_sdk::agent::AgentRunError::Failed {
            cause: kivio_agent_sdk::agent::AgentFailure::LimitExceeded { .. },
            ..
        }
    ));
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 2);
    assert!(seen[0].iter().any(|text| text == "user followup"));
    assert_eq!(seen[0].last().unwrap().trim(), "second");
    assert_eq!(
        *outcomes.lock().unwrap(),
        vec![InjectionSettlement::Applied, InjectionSettlement::Released]
    );
    assert_eq!(model.calls.lock().unwrap().len(), 3);
}

#[tokio::test]
async fn input_arriving_during_collection_releases_finish_reservation() {
    let model = Model::new(vec![output("first"), output("second"), output("third")]);
    let control = Arc::new(Control::default());
    let run = handle(&control);
    let submit = run.clone();
    let outcomes = Arc::new(Mutex::new(vec![]));
    let log = outcomes.clone();
    let mut once = false;
    let mut injections = InjectionSources::new().with(source(move |context| {
        if context.boundary != InjectionBoundary::BeforeFinish || once {
            return Ok(InjectionBatch::default());
        }
        once = true;
        submit.steer(
            kivio_agent_sdk::agent::SteeringMessage::new("steer".into(), "late steer").unwrap(),
        );
        submit.follow_up(
            kivio_agent_sdk::agent::SteeringMessage::new("late".into(), "late followup").unwrap(),
        );
        Ok(reserved("stale reminder", &log))
    }));
    let mut cfg = config(&model, initial_history());
    cfg.tools.clear();
    let result = run_agent_loop(
        cfg,
        &Host,
        Arc::new(EchoTool::new(true)),
        &mut NoSessionPolicy,
        &mut injections,
        &Events::default(),
        run,
    )
    .await
    .unwrap();
    assert_eq!(
        *outcomes.lock().unwrap(),
        vec![InjectionSettlement::Released]
    );
    assert_eq!(result.content, "third");
    assert!(!result
        .history
        .context()
        .iter()
        .any(|m| m.text_content() == "stale reminder"));
    let calls = model.calls.lock().unwrap();
    assert_eq!(calls.len(), 3);
    assert!(calls[1]
        .history
        .iter()
        .any(|m| m.text_content() == "late steer"));
    assert!(!calls[1]
        .history
        .iter()
        .any(|m| m.text_content() == "late followup"));
    assert!(calls[2]
        .history
        .iter()
        .any(|m| m.text_content() == "late followup"));
}

#[tokio::test]
async fn cancellation_after_application_keeps_applied_settlement() {
    let model = Model::new(vec![]);
    let control = Arc::new(Control::default());
    let run = handle(&control);
    let stop = run.clone();
    let outcomes = Arc::new(Mutex::new(vec![]));
    let log = outcomes.clone();
    let mut injections = InjectionSources::new().with(source(move |_| {
        let stop = stop.clone();
        let log = log.clone();
        Ok(
            InjectionBatch::new(vec![input("already applied")]).with_settlement(move |outcome| {
                log.lock().unwrap().push(outcome);
                stop.stop();
            }),
        )
    }));
    let mut cfg = config(&model, initial_history());
    cfg.tools.clear();
    let result = run_agent_loop(
        cfg,
        &Host,
        Arc::new(EchoTool::new(true)),
        &mut NoSessionPolicy,
        &mut injections,
        &Events::default(),
        run,
    )
    .await
    .unwrap();
    assert_eq!(
        *outcomes.lock().unwrap(),
        vec![InjectionSettlement::Applied]
    );
    assert!(result
        .history
        .context()
        .iter()
        .any(|m| m.text_content() == "already applied"));
    assert!(model.calls.lock().unwrap().is_empty());
}
