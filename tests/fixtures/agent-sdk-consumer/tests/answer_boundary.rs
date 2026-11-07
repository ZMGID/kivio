use kivio_agent_sdk::{
    agent::{
        content::ContentKind,
        control::{AgentRunHandle, InputAdmission, StopAdmission},
        events::{AgentEvent, AgentEventKind, AgentEventSink, EventDelivery},
        host::{AgentHost, AgentHostFuture, InputPersistence, SessionInput},
        provider_runtime::{ModelCall, ModelOutputSink, ModelRuntime, ProviderFuture},
        run_agent_loop,
        session::{NoSessionPolicy, PolicyFuture, RunSnapshot, SessionPolicy, TurnDecision},
        types::AgentRunStatus,
        RuntimeMessage, SteeringMessage,
    },
    model::{
        GenerateOutput, GeneratedImageData, MessagePart, ModelError, ModelMessage, ModelRole,
        StreamPart,
    },
};
use kivio_agent_sdk_consumer::{config, handle, initial_history, Control, EchoTool, Events, Host};
use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Mutex,
    },
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

#[tokio::test]
async fn in_flight_steering_preserves_identical_canonical_answers_and_segments_once() {
    let canonical = ModelMessage {
        role: ModelRole::Assistant,
        content: vec![
            MessagePart::AnthropicThinking {
                thinking: "private".into(),
                signature: Some("signed".into()),
                source_model: Some("test-model".into()),
                source_provider_id: Some("explicit-host-provider".into()),
            },
            MessagePart::Text {
                text: "canonical answer".into(),
            },
        ],
    };
    let mut first = output("same answer");
    first.reasoning = Some("first thought".into());
    first.canonical_message = Some(canonical.clone());
    first.images.push(GeneratedImageData {
        mime_type: "image/png".into(),
        data: "aW1hZ2U=".into(),
    });
    let mut second = first.clone();
    second.reasoning = Some("second thought".into());
    second.images.clear();
    let model = Model::new(vec![first, second]);
    let control = Control::default();
    let run = handle(&control);
    let queued = AtomicBool::new(false);
    let events = Events::default();
    let sink = |event: &AgentEvent<'_>| {
        events.on_event(event);
        if matches!(&event.kind, AgentEventKind::Content { delta, .. } if !delta.is_empty())
            && !queued.swap(true, Ordering::SeqCst)
        {
            assert_eq!(
                run.steer(SteeringMessage::new("during".into(), "new direction").unwrap()),
                InputAdmission::Accepted
            );
        }
        EventDelivery::Accepted
    };
    let mut cfg = config(&model, initial_history());
    cfg.tools.clear();
    let result = run_agent_loop(
        cfg,
        &Host,
        &EchoTool::new(true),
        &mut NoSessionPolicy,
        &sink,
        run.clone(),
    )
    .await
    .unwrap();
    let calls = model.calls.lock().unwrap();
    assert_eq!(calls.len(), 2);
    assert_eq!(
        calls[1].history.last().unwrap().text_content(),
        "new direction"
    );
    assert_eq!(
        calls[1].history[calls[1].history.len() - 2].model_message(),
        Some(&canonical)
    );
    for history in [result.history.context(), &result.generated_history] {
        assert_eq!(
            history
                .iter()
                .filter(|message| message.model_message() == Some(&canonical))
                .count(),
            2
        );
        assert_eq!(
            history
                .iter()
                .filter(|message| message.text_content() == "new direction")
                .count(),
            1
        );
    }
    assert_eq!(
        result.reasoning.as_deref(),
        Some("first thought\n\nsecond thought")
    );
    assert_eq!(result.images.len(), 1);
    assert_eq!(
        result
            .segments
            .iter()
            .filter(|block| block.kind == ContentKind::Text)
            .count(),
        2
    );
    assert_eq!(
        result
            .segments
            .iter()
            .filter(|block| block.kind == ContentKind::Reasoning)
            .count(),
        2
    );
    let mut ids = result
        .segments
        .iter()
        .map(|block| block.id)
        .collect::<Vec<_>>();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), result.segments.len());
}

#[derive(Default)]
struct Reminders {
    steps: usize,
    finishes: usize,
}
impl SessionPolicy for Reminders {
    fn prepare_request<'a>(
        &'a mut self,
        _: RunSnapshot<'a>,
        request: &'a mut ModelCall,
        _: AgentRunHandle<'a>,
    ) -> PolicyFuture<'a, Result<(), String>> {
        Box::pin(async move {
            self.steps += 1;
            request.history.push(RuntimeMessage::text(
                ModelRole::User,
                format!("step {}", self.steps),
            ));
            Ok(())
        })
    }
    fn finish_turn<'a>(
        &'a mut self,
        _: RunSnapshot<'a>,
        _: AgentRunHandle<'a>,
    ) -> PolicyFuture<'a, Result<TurnDecision, String>> {
        Box::pin(async move {
            self.finishes += 1;
            Ok(if self.finishes == 1 {
                TurnDecision::Continue(vec![RuntimeMessage::text(
                    ModelRole::User,
                    "finish reminder",
                )])
            } else {
                TurnDecision::Default
            })
        })
    }
}

#[tokio::test]
async fn plain_policy_reminders_continue_at_answer_boundaries() {
    let model = Model::new(vec![output("first"), output("last")]);
    let control = Control::default();
    let mut cfg = config(&model, initial_history());
    cfg.tools.clear();
    let mut policy = Reminders::default();
    let result = run_agent_loop(
        cfg,
        &Host,
        &EchoTool::new(true),
        &mut policy,
        &Events::default(),
        handle(&control),
    )
    .await
    .unwrap();
    assert_eq!((policy.steps, policy.finishes), (2, 2));
    assert_eq!(result.content, "last");
    let calls = model.calls.lock().unwrap();
    assert_eq!(calls[0].history.last().unwrap().text_content(), "step 1");
    let texts = calls[1]
        .history
        .iter()
        .map(RuntimeMessage::text_content)
        .collect::<Vec<_>>();
    assert_eq!(
        &texts[texts.len() - 3..],
        &["first", "finish reminder", "step 2"]
    );
}

struct Deliveries {
    finishes: AtomicUsize,
    waited: AtomicBool,
    child_delivered: AtomicBool,
}
impl AgentHost for Deliveries {
    fn checkpoint_runtime<'a>(
        &'a self,
        _: &'a str,
        _: &'a str,
        _: &'a kivio_agent_sdk::agent::SessionHistory,
        finishing: bool,
    ) -> AgentHostFuture<'a, Result<Vec<SessionInput>, String>> {
        Box::pin(async move {
            if finishing && self.finishes.fetch_add(1, Ordering::SeqCst) == 1 {
                return Ok(vec![
                    SessionInput::append(RuntimeMessage::text(ModelRole::User, "append")),
                    SessionInput {
                        message: RuntimeMessage::text(ModelRole::User, "persisted"),
                        persistence: InputPersistence::AlreadyPersisted,
                    },
                ]);
            }
            if !finishing
                && self.waited.load(Ordering::SeqCst)
                && !self.child_delivered.swap(true, Ordering::SeqCst)
            {
                return Ok(vec![SessionInput::append(RuntimeMessage::text(
                    ModelRole::User,
                    "child result",
                ))]);
            }
            Ok(vec![])
        })
    }
    fn wait_for_child_results<'a>(
        &'a self,
        _: &'a str,
        _: &'a str,
        _: u64,
    ) -> AgentHostFuture<'a, Result<bool, String>> {
        Box::pin(async { Ok(!self.waited.swap(true, Ordering::SeqCst)) })
    }
}

#[tokio::test]
async fn plain_host_delivery_and_child_wait_preserve_persistence_ownership() {
    let model = Model::new(vec![output("first"), output("second"), output("last")]);
    let control = Control::default();
    let mut cfg = config(&model, initial_history());
    cfg.tools.clear();
    let host = Deliveries {
        finishes: AtomicUsize::new(0),
        waited: AtomicBool::new(false),
        child_delivered: AtomicBool::new(false),
    };
    let result = run_agent_loop(
        cfg,
        &host,
        &EchoTool::new(true),
        &mut NoSessionPolicy,
        &Events::default(),
        handle(&control),
    )
    .await
    .unwrap();
    let calls = model.calls.lock().unwrap();
    assert_eq!(calls.len(), 3);
    assert_eq!(calls[1].history.last().unwrap().text_content(), "persisted");
    assert_eq!(
        calls[2].history.last().unwrap().text_content(),
        "child result"
    );
    for text in ["append", "persisted", "child result"] {
        assert_eq!(
            result
                .history
                .context()
                .iter()
                .filter(|message| message.text_content() == text)
                .count(),
            1
        );
        assert_eq!(
            result
                .generated_history
                .iter()
                .filter(|message| message.text_content() == text)
                .count(),
            usize::from(text != "persisted")
        );
    }
}

struct CancelInput;
impl SessionPolicy for CancelInput {
    fn input<'a>(
        &'a mut self,
        _: &'a str,
        control: AgentRunHandle<'a>,
    ) -> PolicyFuture<'a, Result<Vec<RuntimeMessage>, String>> {
        Box::pin(async move {
            assert_eq!(control.stop(), StopAdmission::Requested);
            Ok(vec![])
        })
    }
}

#[tokio::test]
async fn cancellation_during_plain_input_admission_makes_no_request() {
    let model = Model::new(vec![]);
    let control = Control::default();
    let run = handle(&control);
    run.steer(SteeringMessage::new("cancel".into(), "cancel in hook").unwrap());
    run.follow_up(SteeringMessage::new("queued".into(), "do not consume").unwrap());
    let mut cfg = config(&model, initial_history());
    cfg.tools.clear();
    let result = run_agent_loop(
        cfg,
        &Host,
        &EchoTool::new(true),
        &mut CancelInput,
        &Events::default(),
        run,
    )
    .await
    .unwrap();
    assert_eq!(result.status, AgentRunStatus::Cancelled);
    assert!(model.calls.lock().unwrap().is_empty());
    assert!(!result
        .history
        .context()
        .iter()
        .any(|message| message.text_content() == "do not consume"));
}

struct CancelFinish<'a>(AgentRunHandle<'a>);

impl SessionPolicy for CancelFinish<'_> {
    fn finish_turn<'a>(
        &'a mut self,
        _: RunSnapshot<'a>,
        _: AgentRunHandle<'a>,
    ) -> PolicyFuture<'a, Result<TurnDecision, String>> {
        Box::pin(async move {
            assert_eq!(self.0.stop(), StopAdmission::Requested);
            Ok(TurnDecision::Default)
        })
    }
}

#[tokio::test]
async fn cancellation_in_finish_policy_retains_answer_and_cancelled_outcome() {
    for with_tools in [false, true] {
        let model = Model::new(vec![output("completed")]);
        let control = Control::default();
        let run = handle(&control);
        let mut cfg = config(&model, initial_history());
        if !with_tools {
            cfg.tools.clear();
        }
        let result = run_agent_loop(
            cfg,
            &Host,
            &EchoTool::new(true),
            &mut CancelFinish(run.clone()),
            &Events::default(),
            run,
        )
        .await
        .unwrap();
        assert_eq!(result.status, AgentRunStatus::Cancelled);
        assert_eq!(result.content, "completed");
        assert_eq!(model.calls.lock().unwrap().len(), 1);
        assert_eq!(
            result
                .history
                .context()
                .iter()
                .filter(|message| message.role() == Some(ModelRole::Assistant))
                .count(),
            1
        );
    }
}

struct CancelWait<'a>(AgentRunHandle<'a>);

impl AgentHost for CancelWait<'_> {
    fn wait_for_child_results<'a>(
        &'a self,
        _: &'a str,
        _: &'a str,
        _: u64,
    ) -> AgentHostFuture<'a, Result<bool, String>> {
        Box::pin(async {
            self.0.stop();
            Ok(false)
        })
    }
}

#[tokio::test]
async fn cancellation_during_child_wait_keeps_completed_answer_once() {
    let mut answer = output("completed");
    answer.reasoning = Some("thought".into());
    let model = Model::new(vec![answer]);
    let control = Control::default();
    let run = handle(&control);
    let mut cfg = config(&model, initial_history());
    cfg.tools.clear();
    let events = Events::default();
    let result = run_agent_loop(
        cfg,
        &CancelWait(run.clone()),
        &EchoTool::new(true),
        &mut NoSessionPolicy,
        &events,
        run,
    )
    .await
    .unwrap();
    assert_eq!(result.status, AgentRunStatus::Cancelled);
    assert_eq!(result.content, "completed");
    assert_eq!(
        result
            .history
            .context()
            .iter()
            .filter(|message| message.role() == Some(ModelRole::Assistant))
            .count(),
        1
    );
    assert_eq!(model.calls.lock().unwrap().len(), 1);
    assert_eq!(
        events
            .0
            .lock()
            .unwrap()
            .iter()
            .filter(|event| event.kind.starts_with("end:"))
            .count(),
        1
    );
}

struct CancelCheckpoint<'a> {
    control: AgentRunHandle<'a>,
    finishes: AtomicUsize,
}
impl AgentHost for CancelCheckpoint<'_> {
    fn checkpoint_runtime<'a>(
        &'a self,
        _: &'a str,
        _: &'a str,
        _: &'a kivio_agent_sdk::agent::SessionHistory,
        finishing: bool,
    ) -> AgentHostFuture<'a, Result<Vec<SessionInput>, String>> {
        Box::pin(async move {
            if finishing && self.finishes.fetch_add(1, Ordering::SeqCst) == 1 {
                self.control.stop();
                return Ok(vec![SessionInput::append(RuntimeMessage::text(
                    ModelRole::User,
                    "delivered at stop",
                ))]);
            }
            Ok(vec![])
        })
    }
}

#[tokio::test]
async fn cancellation_at_finishing_checkpoint_retains_answer_then_delivered_input() {
    for canonical_output in [false, true] {
        let mut answer = GenerateOutput::text("completed".into(), None);
        answer.reasoning = Some("thought".into());
        let canonical = answer.replay_message();
        if canonical_output {
            answer.canonical_message = Some(canonical.clone());
        }
        let model = Model::new(vec![answer]);
        let control = Control::default();
        let run = handle(&control);
        run.follow_up(SteeringMessage::new("queued".into(), "do not consume").unwrap());
        let mut cfg = config(&model, initial_history());
        cfg.tools.clear();
        let host = CancelCheckpoint {
            control: run.clone(),
            finishes: AtomicUsize::new(0),
        };
        let result = run_agent_loop(
            cfg,
            &host,
            &EchoTool::new(true),
            &mut NoSessionPolicy,
            &Events::default(),
            run,
        )
        .await
        .unwrap();
        assert_eq!(result.status, AgentRunStatus::Cancelled);
        assert_eq!(model.calls.lock().unwrap().len(), 1);
        for history in [result.history.context(), &result.generated_history] {
            assert_eq!(history[history.len() - 2].model_message(), Some(&canonical));
            assert_eq!(history.last().unwrap().text_content(), "delivered at stop");
            assert_eq!(
                history
                    .iter()
                    .filter(|message| message.role() == Some(ModelRole::Assistant))
                    .count(),
                1
            );
            assert!(!history
                .iter()
                .any(|message| message.text_content() == "do not consume"));
        }
    }
}

#[tokio::test]
async fn cancellation_in_follow_up_hook_does_not_reconstruct_prior_reasoning_as_an_answer() {
    let mut answer = output("completed");
    answer.reasoning = Some("thought".into());
    let canonical = answer.replay_message();
    answer.canonical_message = Some(canonical.clone());
    let model = Model::new(vec![answer]);
    let control = Control::default();
    let run = handle(&control);
    run.follow_up(SteeringMessage::new("cancel".into(), "cancel in hook").unwrap());
    let mut cfg = config(&model, initial_history());
    cfg.tools.clear();
    let result = run_agent_loop(
        cfg,
        &Host,
        &EchoTool::new(true),
        &mut CancelInput,
        &Events::default(),
        run,
    )
    .await
    .unwrap();
    assert_eq!(result.status, AgentRunStatus::Cancelled);
    assert_eq!(model.calls.lock().unwrap().len(), 1);
    for history in [result.history.context(), &result.generated_history] {
        let assistants = history
            .iter()
            .filter(|message| message.role() == Some(ModelRole::Assistant))
            .collect::<Vec<_>>();
        assert_eq!(assistants.len(), 1);
        assert_eq!(assistants[0].model_message(), Some(&canonical));
    }
}

#[tokio::test]
async fn queued_plain_inputs_keep_one_per_step_and_steer_precedes_follow_up() {
    let model = Model::new(vec![
        output("one"),
        output("two"),
        output("three"),
        output("four"),
    ]);
    let control = Control::default();
    let run = handle(&control);
    for text in ["steer one", "steer two"] {
        assert_eq!(
            run.steer(SteeringMessage::new(text.into(), text).unwrap()),
            InputAdmission::Accepted
        );
    }
    for text in ["follow one", "follow two"] {
        assert_eq!(
            run.follow_up(SteeringMessage::new(text.into(), text).unwrap()),
            InputAdmission::Accepted
        );
    }
    let mut cfg = config(&model, initial_history());
    cfg.tools.clear();
    let result = run_agent_loop(
        cfg,
        &Host,
        &EchoTool::new(true),
        &mut NoSessionPolicy,
        &Events::default(),
        run,
    )
    .await
    .unwrap();
    let calls = model.calls.lock().unwrap();
    assert_eq!(calls.len(), 4);
    for (call, text) in calls
        .iter()
        .zip(["steer one", "steer two", "follow one", "follow two"])
    {
        assert_eq!(call.history.last().unwrap().text_content(), text);
        assert_eq!(
            result
                .history
                .context()
                .iter()
                .filter(|message| message.text_content() == text)
                .count(),
            1
        );
    }
    assert_eq!(result.content, "four");
}
