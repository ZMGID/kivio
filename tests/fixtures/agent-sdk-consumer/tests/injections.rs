use kivio_agent_sdk::{
    agent::{
        host::{InputPersistence, SessionInput},
        provider_runtime::{ModelCall, ModelOutputSink, ModelRuntime, ProviderFuture},
        run_agent_loop,
        session::NoSessionPolicy,
        InjectionAdmission, InjectionBatch, InjectionBoundary, InjectionContext, InjectionError,
        InjectionFuture, InjectionSettlement, InjectionSource, InjectionSources, RuntimeMessage,
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
    fn collect<'a>(
        &'a mut self,
        context: InjectionContext<'a>,
        admission: &'a mut InjectionAdmission,
    ) -> InjectionFuture<'a, Result<(), InjectionError>> {
        Box::pin(async move {
            admission.pending((self.0)(context).map_err(InjectionError::Source)?);
            Ok(())
        })
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
async fn failed_invalid_panicking_and_oversized_sources_isolate_reservations() {
    for mode in 0..5 {
        let model = Model::new(vec![output("done")]);
        let control = Arc::new(Control::default());
        let outcomes = Arc::new(Mutex::new(vec![]));
        let first = outcomes.clone();
        let second = outcomes.clone();
        let mut injections =
            InjectionSources::new()
                .with(source(move |context| {
                    if context.boundary == InjectionBoundary::BeforeStep {
                        Ok(reserved("first", &first))
                    } else {
                        Ok(InjectionBatch::default())
                    }
                }))
                .with(source(move |context| {
                    if context.boundary != InjectionBoundary::BeforeStep {
                        return Ok(InjectionBatch::default());
                    }
                    match mode {
                        0 => Err("source failed".into()),
                        1 => panic!("source panic"),
                        2 => {
                            let log = second.clone();
                            Ok(InjectionBatch::new(vec![SessionInput::append(
                                RuntimeMessage::system("bad"),
                            )])
                            .with_settlement(move |o| log.lock().unwrap().push(o)))
                        }
                        _ => Ok(reserved("second", &second)),
                    }
                }))
                .with_limits(
                    if mode == 3 { 1 } else { 64 },
                    if mode == 4 { 1 } else { 1024 * 1024 },
                );
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
            result
                .history
                .context()
                .iter()
                .any(|m| m.text_content() == "first"),
            mode != 4
        );
        assert!(!result
            .history
            .context()
            .iter()
            .any(|m| m.text_content() == "second"));
        let settled = outcomes.lock().unwrap();
        assert_eq!(
            settled[0],
            if mode == 4 {
                InjectionSettlement::Released
            } else {
                InjectionSettlement::Applied
            }
        );
        assert!(settled
            .iter()
            .skip(1)
            .all(|o| *o == InjectionSettlement::Released));
        assert_eq!(model.calls.lock().unwrap().len(), 1);
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

use kivio_agent_sdk::agent::{
    DropReason, InjectionKind, InjectionOutcome, InjectionQueue, InjectionSender,
    PreparedInjection, WakePolicy,
};
use std::sync::atomic::{AtomicUsize, Ordering};

struct TextKind {
    version: Arc<AtomicUsize>,
    action: Option<Box<dyn Fn() + Send + Sync>>,
}
impl InjectionKind for TextKind {
    type Item = (usize, String);
    fn is_stale(&self, item: &Self::Item, _: InjectionContext<'_>) -> bool {
        item.0 != self.version.load(Ordering::SeqCst)
    }
    fn build(
        &self,
        items: &[Self::Item],
        _: InjectionContext<'_>,
    ) -> Result<Vec<SessionInput>, InjectionError> {
        if let Some(action) = &self.action {
            action();
        }
        Ok(vec![input(
            &items
                .iter()
                .map(|item| item.1.as_str())
                .collect::<Vec<_>>()
                .join(" + "),
        )])
    }
}
struct NumberKind;
impl InjectionKind for NumberKind {
    type Item = u32;
    fn build(
        &self,
        items: &[u32],
        _: InjectionContext<'_>,
    ) -> Result<Vec<SessionInput>, InjectionError> {
        Ok(vec![input(&format!("sum {}", items.iter().sum::<u32>()))])
    }
}
fn text_kind(version: Arc<AtomicUsize>) -> TextKind {
    TextKind {
        version,
        action: None,
    }
}

async fn run_sources(
    sources: &mut InjectionSources,
) -> (
    kivio_agent_sdk::agent::types::AgentRunResult,
    Vec<ModelCall>,
) {
    let model = Model::new(vec![output("done")]);
    let control = Arc::new(Control::default());
    let mut cfg = config(&model, initial_history());
    cfg.tools.clear();
    let result = run_agent_loop(
        cfg,
        &Host,
        Arc::new(EchoTool::new(true)),
        &mut NoSessionPolicy,
        sources,
        &Events::default(),
        handle(&control),
    )
    .await
    .unwrap();
    let calls = model.calls.into_inner().unwrap();
    (result, calls)
}

#[tokio::test]
async fn typed_queue_batches_each_kind_and_settles_individual_receipts() {
    let queue = InjectionQueue::new();
    let mut changes = queue.subscribe();
    let words = queue
        .register(text_kind(Arc::new(AtomicUsize::new(2))))
        .unwrap();
    let numbers = queue.register(NumberKind).unwrap();
    let stale = words
        .enqueue((1, "obsolete".into()), WakePolicy::AtBoundary)
        .unwrap();
    let one = words
        .enqueue((2, "one".into()), WakePolicy::AtBoundary)
        .unwrap();
    assert!(queue.has_pending());
    assert!(!queue.has_waking_input());
    let two = words
        .enqueue((2, "two".into()), WakePolicy::WhenIdle)
        .unwrap();
    let n1 = numbers.enqueue(3, WakePolicy::AtBoundary).unwrap();
    let n2 = numbers.enqueue(4, WakePolicy::AtBoundary).unwrap();
    assert!(queue.has_waking_input());
    changes.changed().await.unwrap();
    let (result, calls) = run_sources(&mut InjectionSources::new().with(queue.clone())).await;
    assert_eq!(calls.len(), 1);
    let texts = calls[0]
        .history
        .iter()
        .map(RuntimeMessage::text_content)
        .collect::<Vec<_>>();
    assert_eq!(&texts[texts.len() - 2..], &["one + two", "sum 7"]);
    assert!(!result
        .history
        .context()
        .iter()
        .any(|message| message.text_content().contains("obsolete")));
    assert_eq!(
        stale.wait().await,
        InjectionOutcome::Dropped(DropReason::Stale)
    );
    for receipt in [one, two, n1, n2] {
        assert_eq!(receipt.wait().await, InjectionOutcome::Applied);
    }
    assert!(!queue.has_pending());
    assert!(!queue.has_waking_input());
}

struct ReservedQueue {
    queue: InjectionQueue,
    boundary: InjectionBoundary,
    entered: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Notify>,
    once: bool,
}
impl InjectionSource for ReservedQueue {
    fn collect<'a>(
        &'a mut self,
        context: InjectionContext<'a>,
        admission: &'a mut InjectionAdmission,
    ) -> InjectionFuture<'a, Result<(), InjectionError>> {
        Box::pin(async move {
            if context.boundary != self.boundary {
                return Ok(());
            }
            self.queue.collect(context, admission).await?;
            if !self.once {
                self.once = true;
                self.entered.notify_one();
                self.release.notified().await;
            }
            Ok(())
        })
    }
}

#[tokio::test]
async fn reserved_entries_observe_final_freshness_clear_and_unregister() {
    for mode in 0..3 {
        let queue = InjectionQueue::new();
        let version = Arc::new(AtomicUsize::new(1));
        let sender = queue.register(text_kind(version.clone())).unwrap();
        let receipt = sender
            .enqueue((1, "late".into()), WakePolicy::WhenIdle)
            .unwrap();
        let entered = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let mut sources = InjectionSources::new().with(ReservedQueue {
            queue: queue.clone(),
            boundary: InjectionBoundary::BeforeStep,
            entered: entered.clone(),
            release: release.clone(),
            once: false,
        });
        let ((result, calls), ()) = tokio::join!(run_sources(&mut sources), async {
            entered.notified().await;
            match mode {
                0 => {
                    version.store(2, Ordering::SeqCst);
                }
                1 => sender.clear(),
                _ => sender.unregister(),
            }
            release.notify_one();
        });
        assert!(!calls[0].history.iter().any(|m| m.text_content() == "late"));
        assert!(!result
            .history
            .context()
            .iter()
            .any(|m| m.text_content() == "late"));
        let reason = match mode {
            0 => DropReason::Stale,
            1 => DropReason::Cleared,
            _ => DropReason::Unregistered,
        };
        assert_eq!(receipt.wait().await, InjectionOutcome::Dropped(reason));
        let next = sender.enqueue(
            (version.load(Ordering::SeqCst), "next".into()),
            WakePolicy::AtBoundary,
        );
        if mode == 2 {
            assert!(matches!(next, Err(InjectionError::Unregistered)));
        } else {
            let (_, calls) = run_sources(&mut InjectionSources::new().with(queue)).await;
            assert!(calls[0].history.iter().any(|m| m.text_content() == "next"));
            assert_eq!(next.unwrap().wait().await, InjectionOutcome::Applied);
        }
    }
}

#[tokio::test]
async fn input_arriving_during_finish_collection_preserves_typed_queue_facts() {
    let model = Model::new(vec![output("first"), output("second"), output("third")]);
    let control = Arc::new(Control::default());
    let run = handle(&control);
    let queue = InjectionQueue::new();
    let sender = queue.register(NumberKind).unwrap();
    let receipt = sender.enqueue(5, WakePolicy::WhenIdle).unwrap();
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let mut sources = InjectionSources::new().with(ReservedQueue {
        queue: queue.clone(),
        boundary: InjectionBoundary::BeforeFinish,
        entered: entered.clone(),
        release: release.clone(),
        once: false,
    });
    let mut cfg = config(&model, initial_history());
    cfg.tools.clear();
    let mut policy = NoSessionPolicy;
    let events = Events::default();
    let (result, ()) = tokio::join!(
        run_agent_loop(
            cfg,
            &Host,
            Arc::new(EchoTool::new(true)),
            &mut policy,
            &mut sources,
            &events,
            run.clone(),
        ),
        async {
            entered.notified().await;
            run.steer(
                kivio_agent_sdk::agent::SteeringMessage::new("steer".into(), "late steer").unwrap(),
            );
            run.follow_up(
                kivio_agent_sdk::agent::SteeringMessage::new("follow".into(), "late followup")
                    .unwrap(),
            );
            release.notify_one();
        }
    );
    let result = result.unwrap();
    assert_eq!(receipt.wait().await, InjectionOutcome::Applied);
    assert!(!queue.has_pending());
    assert_eq!(result.content, "third");
    assert_eq!(
        result
            .history
            .context()
            .iter()
            .filter(|m| m.text_content() == "sum 5")
            .count(),
        1
    );
    let calls = model.calls.lock().unwrap();
    assert_eq!(calls.len(), 3);
    assert!(!calls[0].history.iter().any(|m| m.text_content() == "sum 5"));
    for call in &calls[1..] {
        assert_eq!(
            call.history
                .iter()
                .filter(|m| m.text_content() == "sum 5")
                .count(),
            1
        );
        assert!(call
            .history
            .iter()
            .any(|m| m.text_content() == "late steer"));
    }
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
async fn clear_during_builder_prevents_append_without_holding_user_callback_lock() {
    for unregister in [false, true] {
        let queue = InjectionQueue::new();
        let slot = Arc::new(Mutex::new(None::<InjectionSender<(usize, String)>>));
        let action_slot = slot.clone();
        let sender = queue
            .register(TextKind {
                version: Arc::new(AtomicUsize::new(1)),
                action: Some(Box::new(move || {
                    let sender = action_slot.lock().unwrap().as_ref().unwrap().clone();
                    if unregister {
                        sender.unregister();
                    } else {
                        sender.clear();
                    }
                })),
            })
            .unwrap();
        *slot.lock().unwrap() = Some(sender.clone());
        let receipt = sender
            .enqueue((1, "revoked".into()), WakePolicy::WhenIdle)
            .unwrap();
        let (_, calls) = run_sources(&mut InjectionSources::new().with(queue)).await;
        assert!(!calls[0]
            .history
            .iter()
            .any(|m| m.text_content() == "revoked"));
        assert_eq!(
            receipt.wait().await,
            InjectionOutcome::Dropped(if unregister {
                DropReason::Unregistered
            } else {
                DropReason::Cleared
            })
        );
        slot.lock().unwrap().take();
    }
}

struct BrokenKind(u8);
impl InjectionKind for BrokenKind {
    type Item = ();
    fn is_stale(&self, _: &(), _: InjectionContext<'_>) -> bool {
        if self.0 == 0 {
            panic!("predicate");
        }
        false
    }
    fn build(
        &self,
        _: &[()],
        _: InjectionContext<'_>,
    ) -> Result<Vec<SessionInput>, InjectionError> {
        match self.0 {
            1 => panic!("builder"),
            2 => Err(InjectionError::Source("build failed".into())),
            3 => Ok(vec![SessionInput::append(RuntimeMessage::system(
                "invalid",
            ))]),
            _ => Ok(vec![input(&"large".repeat(100))]),
        }
    }
}
#[tokio::test]
async fn failing_kinds_do_not_consume_other_kinds_or_leave_receipts_pending() {
    for mode in 0..5 {
        let queue = InjectionQueue::new();
        let broken = queue.register(BrokenKind(mode)).unwrap();
        let good = queue.register(NumberKind).unwrap();
        let failed = broken.enqueue((), WakePolicy::WhenIdle).unwrap();
        let applied = good.enqueue(9, WakePolicy::WhenIdle).unwrap();
        let (_, calls) = run_sources(
            &mut InjectionSources::new()
                .with(queue.clone())
                .with_limits(2, 300),
        )
        .await;
        assert!(calls[0].history.iter().any(|m| m.text_content() == "sum 9"));
        assert!(matches!(failed.wait().await, InjectionOutcome::Failed(_)));
        assert_eq!(applied.wait().await, InjectionOutcome::Applied);
        assert!(!queue.has_pending());
    }
}

struct DurableSource {
    mode: u8,
    run: kivio_agent_sdk::agent::control::AgentRunHandle,
    entered: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Notify>,
    once: bool,
}
impl InjectionSource for DurableSource {
    fn collect<'a>(
        &'a mut self,
        _: InjectionContext<'a>,
        admission: &'a mut InjectionAdmission,
    ) -> InjectionFuture<'a, Result<(), InjectionError>> {
        Box::pin(async move {
            if self.once {
                return Ok(());
            }
            self.once = true;
            let prepared = PreparedInjection::new(vec![input("committed task")])?;
            if self.mode == 3 {
                self.entered.notify_one();
                self.release.notified().await;
            }
            admission.committed(prepared);
            assert_eq!(admission.committed_inputs().len(), 1);
            match self.mode {
                0 => Err(InjectionError::Persistence("later IRC save failed".into())),
                1 => panic!("later IRC panic"),
                2 => {
                    self.run.stop();
                    Ok(())
                }
                3 => Ok(()),
                _ => Err(InjectionError::Source("later producer failed".into())),
            }
        })
    }
}

#[tokio::test]
async fn committed_facts_survive_same_source_error_panic_stop_and_stop_during_save() {
    for mode in 0..5 {
        let model = Model::new(vec![output("done")]);
        let control = Arc::new(Control::default());
        let run = handle(&control);
        let entered = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let mut sources = InjectionSources::new().with(DurableSource {
            mode,
            run: run.clone(),
            entered: entered.clone(),
            release: release.clone(),
            once: false,
        });
        let mut cfg = config(&model, initial_history());
        cfg.tools.clear();
        let mut policy = NoSessionPolicy;
        let events = Events::default();
        let future = run_agent_loop(
            cfg,
            &Host,
            Arc::new(EchoTool::new(true)),
            &mut policy,
            &mut sources,
            &events,
            run.clone(),
        );
        let result = if mode == 3 {
            let (result, ()) = tokio::join!(future, async {
                entered.notified().await;
                run.stop();
                release.notify_one();
            });
            result
        } else {
            future.await
        };
        let history = match result {
            Ok(result) => result.history,
            Err(kivio_agent_sdk::agent::AgentRunError::Failed { partial, .. }) => partial.history,
            Err(error) => panic!("unexpected {error:?}"),
        };
        assert_eq!(
            history
                .context()
                .iter()
                .filter(|m| m.text_content() == "committed task")
                .count(),
            1
        );
        assert_eq!(
            model.calls.lock().unwrap().len(),
            usize::from(mode == 1 || mode == 4)
        );
    }
}

#[test]
fn durable_inputs_are_validated_before_a_producer_can_commit() {
    assert!(matches!(
        PreparedInjection::new(vec![SessionInput::append(RuntimeMessage::system(
            "invalid"
        ))]),
        Err(InjectionError::InvalidInput(_))
    ));
}

#[tokio::test]
async fn dropping_queue_closes_all_sender_clones_and_settles_pending_entries() {
    let queue = InjectionQueue::new();
    let sender = queue.register(NumberKind).unwrap();
    let clone = sender.clone();
    let receipt = sender.enqueue(1, WakePolicy::WhenIdle).unwrap();
    drop(queue);
    assert_eq!(
        receipt.wait().await,
        InjectionOutcome::Dropped(DropReason::Unregistered)
    );
    assert!(matches!(
        clone.enqueue(2, WakePolicy::AtBoundary),
        Err(InjectionError::Unregistered)
    ));
}

struct CommitAtFinish;
impl InjectionSource for CommitAtFinish {
    fn collect<'a>(
        &'a mut self,
        context: InjectionContext<'a>,
        admission: &'a mut InjectionAdmission,
    ) -> InjectionFuture<'a, Result<(), InjectionError>> {
        Box::pin(async move {
            if context.boundary == InjectionBoundary::BeforeFinish {
                admission.committed(PreparedInjection::new(vec![input("saved beyond budget")])?);
                admission.pending(InjectionBatch::new(vec![input("revocable beyond budget")]));
            }
            Ok(())
        })
    }
}
#[tokio::test]
async fn model_budget_failure_preserves_committed_facts_and_releases_pending_inputs() {
    let model = Model::new(vec![output("answer")]);
    let control = Arc::new(Control::default());
    let mut cfg = config(&model, initial_history());
    cfg.tools.clear();
    cfg.limits.max_model_steps = std::num::NonZeroU32::new(1);
    let error = run_agent_loop(
        cfg,
        &Host,
        Arc::new(EchoTool::new(true)),
        &mut NoSessionPolicy,
        &mut InjectionSources::new()
            .with(CommitAtFinish)
            .with_limits(0, 0),
        &Events::default(),
        handle(&control),
    )
    .await
    .unwrap_err();
    let kivio_agent_sdk::agent::AgentRunError::Failed { partial, .. } = error else {
        panic!("expected budget failure")
    };
    let texts = partial
        .history
        .context()
        .iter()
        .map(RuntimeMessage::text_content)
        .collect::<Vec<_>>();
    assert!(texts.contains(&"saved beyond budget".into()));
    assert!(!texts.contains(&"revocable beyond budget".into()));
    assert_eq!(texts.iter().filter(|text| *text == "answer").count(), 1);
    assert_eq!(model.calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn receipt_waker_can_clear_the_kind_after_append_without_deadlock_or_retraction() {
    struct ClearOnWake(InjectionSender<u32>);
    impl std::task::Wake for ClearOnWake {
        fn wake(self: Arc<Self>) {
            self.0.clear();
        }
    }
    let queue = InjectionQueue::new();
    let sender = queue.register(NumberKind).unwrap();
    let receipt = sender.enqueue(5, WakePolicy::WhenIdle).unwrap();
    let mut waiting = Box::pin(receipt.wait());
    let waker = std::task::Waker::from(Arc::new(ClearOnWake(sender)));
    assert!(std::future::Future::poll(
        waiting.as_mut(),
        &mut std::task::Context::from_waker(&waker)
    )
    .is_pending());
    let (result, calls) = run_sources(&mut InjectionSources::new().with(queue)).await;
    assert_eq!(waiting.await, InjectionOutcome::Applied);
    assert!(calls[0]
        .history
        .iter()
        .any(|message| message.text_content() == "sum 5"));
    assert!(result
        .history
        .context()
        .iter()
        .any(|message| message.text_content() == "sum 5"));
}

struct FinishQueue(InjectionQueue);
impl InjectionSource for FinishQueue {
    fn collect<'a>(
        &'a mut self,
        context: InjectionContext<'a>,
        admission: &'a mut InjectionAdmission,
    ) -> InjectionFuture<'a, Result<(), InjectionError>> {
        Box::pin(async move {
            if context.boundary == InjectionBoundary::BeforeFinish {
                self.0.collect(context, admission).await?;
            }
            Ok(())
        })
    }
}

#[tokio::test]
async fn cleared_finish_notice_does_not_exhaust_model_budget() {
    revoked_finish_notice(DropReason::Cleared).await;
}

#[tokio::test]
async fn unregistered_finish_notice_does_not_exhaust_model_budget() {
    revoked_finish_notice(DropReason::Unregistered).await;
}

async fn revoked_finish_notice(reason: DropReason) {
    let model = Model::new(vec![output("answer")]);
    let control = Arc::new(Control::default());
    let queue = InjectionQueue::new();
    let slot = Arc::new(Mutex::new(None::<InjectionSender<(usize, String)>>));
    let action_slot = slot.clone();
    let sender = queue
        .register(TextKind {
            version: Arc::new(AtomicUsize::new(1)),
            action: Some(Box::new(move || {
                let sender = action_slot.lock().unwrap().as_ref().unwrap().clone();
                if reason == DropReason::Unregistered {
                    sender.unregister();
                } else {
                    sender.clear();
                }
            })),
        })
        .unwrap();
    *slot.lock().unwrap() = Some(sender.clone());
    let receipt = sender
        .enqueue((1, "revoked notice".into()), WakePolicy::AtBoundary)
        .unwrap();
    let mut cfg = config(&model, initial_history());
    cfg.tools.clear();
    cfg.limits.max_model_steps = std::num::NonZeroU32::new(1);
    let result = run_agent_loop(
        cfg,
        &Host,
        Arc::new(EchoTool::new(true)),
        &mut NoSessionPolicy,
        &mut InjectionSources::new().with(FinishQueue(queue)),
        &Events::default(),
        handle(&control),
    )
    .await;
    slot.lock().unwrap().take();
    assert_eq!(receipt.wait().await, InjectionOutcome::Dropped(reason));
    let result = result.expect("revoked input must not fail a completed answer");
    assert_eq!(result.content, "answer");
    assert!(!result
        .history
        .context()
        .iter()
        .any(|m| m.text_content() == "revoked notice"));
    assert_eq!(model.calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn live_finish_notice_still_checks_budget_and_releases_receipt() {
    use kivio_agent_sdk::agent::{error::RunLimit, AgentFailure, AgentRunError};
    let model = Model::new(vec![output("answer")]);
    let control = Arc::new(Control::default());
    let queue = InjectionQueue::new();
    let sender = queue.register(NumberKind).unwrap();
    let receipt = sender.enqueue(5, WakePolicy::AtBoundary).unwrap();
    let mut cfg = config(&model, initial_history());
    cfg.tools.clear();
    cfg.limits.max_model_steps = std::num::NonZeroU32::new(1);
    let error = run_agent_loop(
        cfg,
        &Host,
        Arc::new(EchoTool::new(true)),
        &mut NoSessionPolicy,
        &mut InjectionSources::new().with(FinishQueue(queue.clone())),
        &Events::default(),
        handle(&control),
    )
    .await
    .unwrap_err();
    let AgentRunError::Failed {
        cause: AgentFailure::LimitExceeded {
            limit: RunLimit::ModelSteps,
        },
        partial,
    } = error
    else {
        panic!("unexpected failure: {error:?}")
    };
    assert_eq!(
        receipt.wait().await,
        InjectionOutcome::Dropped(DropReason::Released)
    );
    assert!(!queue.has_pending());
    assert!(!partial
        .history
        .context()
        .iter()
        .any(|m| m.text_content() == "sum 5"));
    assert_eq!(
        partial
            .history
            .context()
            .iter()
            .filter(|m| m.text_content() == "answer")
            .count(),
        1
    );
    assert_eq!(model.calls.lock().unwrap().len(), 1);
}

struct SteerDuringSource {
    run: kivio_agent_sdk::agent::control::AgentRunHandle,
    stop: bool,
    once: bool,
}
impl InjectionSource for SteerDuringSource {
    fn collect<'a>(
        &'a mut self,
        context: InjectionContext<'a>,
        _: &'a mut InjectionAdmission,
    ) -> InjectionFuture<'a, Result<(), InjectionError>> {
        Box::pin(async move {
            if context.boundary == InjectionBoundary::BeforeStep && !self.once {
                self.once = true;
                tokio::task::yield_now().await;
                self.run.steer(
                    kivio_agent_sdk::agent::SteeringMessage::new("late".into(), "late steering")
                        .unwrap(),
                );
                if self.stop {
                    self.run.stop();
                }
            }
            Ok(())
        })
    }
}

#[tokio::test]
async fn steering_during_before_step_precedes_next_model_call() {
    let model = Model::new(vec![output("answer"), output("unnecessary second answer")]);
    let control = Arc::new(Control::default());
    let run = handle(&control);
    let mut cfg = config(&model, initial_history());
    cfg.tools.clear();
    run_agent_loop(
        cfg,
        &Host,
        Arc::new(EchoTool::new(true)),
        &mut NoSessionPolicy,
        &mut InjectionSources::new().with(SteerDuringSource {
            run: run.clone(),
            stop: false,
            once: false,
        }),
        &Events::default(),
        run,
    )
    .await
    .unwrap();
    let calls = model.calls.lock().unwrap();
    assert!(calls[0]
        .history
        .iter()
        .any(|m| m.text_content() == "late steering"));
    assert_eq!(calls.len(), 1);
}

#[tokio::test]
async fn steering_consumes_one_per_boundary_in_fifo_order_even_with_late_arrival() {
    let model = Model::new(vec![
        output("first answer"),
        output("second answer"),
        output("third answer"),
    ]);
    let control = Arc::new(Control::default());
    let run = handle(&control);
    for text in ["first steering", "second steering"] {
        run.steer(kivio_agent_sdk::agent::SteeringMessage::new(text.into(), text).unwrap());
    }
    let mut cfg = config(&model, initial_history());
    cfg.tools.clear();
    run_agent_loop(
        cfg,
        &Host,
        Arc::new(EchoTool::new(true)),
        &mut NoSessionPolicy,
        &mut InjectionSources::new().with(SteerDuringSource {
            run: run.clone(),
            stop: false,
            once: false,
        }),
        &Events::default(),
        run,
    )
    .await
    .unwrap();
    let calls = model.calls.lock().unwrap();
    assert_eq!(calls.len(), 3);
    let expected = ["first steering", "second steering", "late steering"];
    for (index, call) in calls.iter().enumerate() {
        let steering = call
            .history
            .iter()
            .map(RuntimeMessage::text_content)
            .filter(|text| expected.contains(&text.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(steering, expected[..=index]);
    }
}

#[tokio::test]
async fn stop_during_source_does_not_consume_late_steering() {
    let model = Model::new(vec![]);
    let control = Arc::new(Control::default());
    let run = handle(&control);
    let mut cfg = config(&model, initial_history());
    cfg.tools.clear();
    let result = run_agent_loop(
        cfg,
        &Host,
        Arc::new(EchoTool::new(true)),
        &mut NoSessionPolicy,
        &mut InjectionSources::new().with(SteerDuringSource {
            run: run.clone(),
            stop: true,
            once: false,
        }),
        &Events::default(),
        run,
    )
    .await
    .unwrap();
    assert!(!result
        .history
        .context()
        .iter()
        .any(|m| m.text_content() == "late steering"));
    assert!(model.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn steering_during_checkpoint_precedes_next_model_call() {
    struct CheckpointSteer {
        run: kivio_agent_sdk::agent::control::AgentRunHandle,
        calls: AtomicUsize,
    }
    impl kivio_agent_sdk::agent::AgentHost for CheckpointSteer {
        fn checkpoint_runtime<'a>(
            &'a self,
            _: &'a str,
            _: &'a str,
            _: &'a kivio_agent_sdk::agent::SessionHistory,
            _: bool,
        ) -> kivio_agent_sdk::agent::AgentHostFuture<'a, Result<(), String>> {
            Box::pin(async move {
                if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
                    tokio::task::yield_now().await;
                    self.run.steer(
                        kivio_agent_sdk::agent::SteeringMessage::new(
                            "checkpoint".into(),
                            "steering during checkpoint",
                        )
                        .unwrap(),
                    );
                }
                Ok(())
            })
        }
    }
    let model = Model::new(vec![output("answer"), output("unnecessary second answer")]);
    let control = Arc::new(Control::default());
    let run = handle(&control);
    let host = CheckpointSteer {
        run: run.clone(),
        calls: AtomicUsize::new(0),
    };
    let mut cfg = config(&model, initial_history());
    cfg.tools.clear();
    run_agent_loop(
        cfg,
        &host,
        Arc::new(EchoTool::new(true)),
        &mut NoSessionPolicy,
        &mut InjectionSources::new(),
        &Events::default(),
        run,
    )
    .await
    .unwrap();
    let calls = model.calls.lock().unwrap();
    assert!(calls[0]
        .history
        .iter()
        .any(|m| m.text_content() == "steering during checkpoint"));
    assert_eq!(calls.len(), 1);
}

#[tokio::test]
async fn budget_timeout_releases_receipt_before_reentrant_stop_callback() {
    use kivio_agent_sdk::agent::{
        control::{AgentRunControl, AgentRunHandle, InputAdmission, StopAdmission},
        error::RunLimit,
        events::InputKind,
        AgentFailure, AgentHostFuture, AgentRunError, SteeringMessage,
    };
    use std::time::Duration;

    struct ClearOnStop {
        control: Control,
        sender: InjectionSender<(usize, String)>,
        stops: AtomicUsize,
    }
    impl AgentRunControl for ClearOnStop {
        fn active(&self) -> bool {
            self.control.active()
        }
        fn stop(&self) -> StopAdmission {
            self.stops.fetch_add(1, Ordering::SeqCst);
            let sender = self.sender.clone();
            let (done, wait) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                sender.clear();
                done.send(()).unwrap();
            });
            wait.recv_timeout(Duration::from_secs(2))
                .expect("Stop callback holds the queue lock");
            self.control.stop()
        }
        fn submit(&self, kind: InputKind, message: SteeringMessage) -> InputAdmission {
            self.control.submit(kind, message)
        }
        fn take(&self, kind: InputKind) -> Vec<SteeringMessage> {
            self.control.take(kind)
        }
        fn wait_inactive(&self) -> AgentHostFuture<'_, ()> {
            self.control.wait_inactive()
        }
    }

    let model = Model::new(vec![output("answer")]);
    let queue = InjectionQueue::new();
    let builds = Arc::new(AtomicUsize::new(0));
    let built = builds.clone();
    let sender = queue
        .register(TextKind {
            version: Arc::new(AtomicUsize::new(1)),
            action: Some(Box::new(move || {
                built.fetch_add(1, Ordering::SeqCst);
                // The synchronous builder crosses the deadline before the outer timer can be polled.
                std::thread::sleep(Duration::from_millis(600));
            })),
        })
        .unwrap();
    let receipt = sender
        .enqueue((1, "overdue notice".into()), WakePolicy::AtBoundary)
        .unwrap();
    let control = Arc::new(ClearOnStop {
        control: Control::default(),
        sender,
        stops: AtomicUsize::new(0),
    });
    let identity = handle(&Arc::new(Control::default()));
    let run = AgentRunHandle::new(identity.identity(), control.clone());
    let mut cfg = config(&model, initial_history());
    cfg.tools.clear();
    cfg.limits.timeout = Some(Duration::from_millis(500));
    let error = run_agent_loop(
        cfg,
        &Host,
        Arc::new(EchoTool::new(true)),
        &mut NoSessionPolicy,
        &mut InjectionSources::new().with(FinishQueue(queue.clone())),
        &Events::default(),
        run,
    )
    .await
    .unwrap_err();
    let AgentRunError::Failed {
        cause: AgentFailure::LimitExceeded {
            limit: RunLimit::Timeout,
        },
        partial,
    } = error
    else {
        panic!("unexpected failure: {error:?}")
    };
    assert_eq!(builds.load(Ordering::SeqCst), 1);
    assert_eq!(control.stops.load(Ordering::SeqCst), 1);
    assert_eq!(
        receipt.wait().await,
        InjectionOutcome::Dropped(DropReason::Released)
    );
    assert!(!queue.has_pending());
    assert!(!partial
        .history
        .context()
        .iter()
        .any(|m| m.text_content() == "overdue notice"));
    assert_eq!(model.calls.lock().unwrap().len(), 1);
}
