#[allow(dead_code)]
#[path = "../examples/support/mod.rs"]
mod support;

use kivio_agent_sdk::{
    agent::{
        content::ContentKind,
        control::AgentRunHandle,
        events::{AgentEvent, EventDelivery},
        provider_runtime::{ModelCall, ModelOutputSink, ModelRuntime, ProviderFuture},
        recovery::{RecoveryDecision, RecoverySnapshot},
        run_agent_loop,
        session::{NoSessionPolicy, PolicyFuture, RunSnapshot, SessionPolicy, TurnDecision},
        types::{AgentRunResult, AgentRunStatus},
        InjectionSources,
    },
    model::{
        FinishReason, GenerateOutput, ModelError, ModelErrorKind, ModelUsage, PendingToolCall,
    },
};
use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
};

struct Model {
    calls: AtomicUsize,
    outputs: Mutex<VecDeque<Result<GenerateOutput, ModelError>>>,
}
impl Model {
    fn new(outputs: Vec<Result<GenerateOutput, ModelError>>) -> Self {
        Self {
            calls: AtomicUsize::new(0),
            outputs: Mutex::new(outputs.into()),
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
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.outputs
                .lock()
                .unwrap()
                .pop_front()
                .expect("unexpected model request")
        })
    }
}
fn text(value: &str) -> GenerateOutput {
    GenerateOutput::text(value.into(), None)
}
fn tool() -> GenerateOutput {
    let mut output = text("");
    output.tool_calls.push(PendingToolCall {
        id: "echo-1".into(),
        function_name: "echo".into(),
        arguments: serde_json::json!({"text":"partial work"}),
        arguments_raw: r#"{"text":"partial work"}"#.into(),
        arguments_parse_error: None,
        signature: None,
    });
    output
}
async fn run(model: &Model, policy: &mut dyn SessionPolicy) -> AgentRunResult {
    let control = Arc::new(support::Control::default());
    run_agent_loop(
        support::config(model, support::initial_history()),
        &support::Host,
        Arc::new(support::EchoTool::new(true)),
        policy,
        &mut InjectionSources::new(),
        &|_: &AgentEvent<'_>| EventDelivery::Accepted,
        support::handle(&control),
    )
    .await
    .unwrap()
}
#[derive(Default)]
struct Recovery {
    continue_after: bool,
    errors: Vec<ModelError>,
}
impl SessionPolicy for Recovery {
    fn recover(&mut self, _: RecoverySnapshot<'_>, error: &ModelError) -> RecoveryDecision {
        self.errors.push(error.clone());
        RecoveryDecision::Complete("temporary failure notice".into())
    }
    fn finish_turn<'a>(
        &'a mut self,
        run: RunSnapshot<'a>,
        _: AgentRunHandle,
    ) -> PolicyFuture<'a, Result<TurnDecision, String>> {
        Box::pin(async move {
            Ok(
                if self.continue_after
                    && run
                        .history
                        .last()
                        .is_some_and(|m| m.text_content() == "temporary failure notice")
                {
                    TurnDecision::Continue
                } else {
                    TurnDecision::Default
                },
            )
        })
    }
}

#[tokio::test]
async fn empty_planning_variants_retry_and_account_for_both_responses() {
    for (body, reasoning) in [
        ("", None),
        ("", Some("thinking")),
        ("<|DSML|tool_calls></|DSML|tool_calls>", None),
    ] {
        let mut empty = GenerateOutput::text(body.into(), reasoning.map(str::to_owned));
        empty.usage = Some(ModelUsage {
            input_tokens: Some(10),
            output_tokens: Some(2),
            ..Default::default()
        });
        let mut answer = text("retry answer");
        answer.usage = Some(ModelUsage {
            input_tokens: Some(20),
            output_tokens: Some(3),
            ..Default::default()
        });
        let model = Model::new(vec![Ok(empty), Ok(answer)]);
        let result = run(&model, &mut NoSessionPolicy).await;
        assert_eq!(model.calls.load(Ordering::SeqCst), 2);
        assert_eq!(result.content, "retry answer");
        assert_eq!(result.status, AgentRunStatus::Completed);
        let usage = result.usage.unwrap();
        assert_eq!(
            (usage.input_tokens, usage.output_tokens),
            (Some(30), Some(5))
        );
    }
}

#[tokio::test]
async fn consecutive_empty_planning_uses_existing_recovery_with_diagnostics() {
    let mut empty = GenerateOutput::text("".into(), Some("thinking".into()));
    empty.finish_reason = Some(FinishReason::from_openai("stop"));
    empty.usage = Some(ModelUsage {
        input_tokens: Some(20),
        output_tokens: Some(2),
        ..Default::default()
    });
    let model = Model::new(vec![Ok(tool()), Ok(empty.clone()), Ok(empty)]);
    let mut policy = Recovery::default();
    let result = run(&model, &mut policy).await;
    assert_eq!(model.calls.load(Ordering::SeqCst), 3);
    assert_eq!(result.status, AgentRunStatus::Recovered);
    assert_eq!(result.content, "temporary failure notice");
    assert_eq!(policy.errors.len(), 1);
    let error = &policy.errors[0];
    assert_eq!(error.kind, ModelErrorKind::EmptyResponse);
    for part in [
        "empty assistant response",
        "finish_reason=Stop",
        "text_chars=0",
        "reasoning_chars=8",
        "tool_calls=0",
        "tool_drafts=0",
        "input_tokens=20",
        "output_tokens=2",
    ] {
        assert!(
            error.message.contains(part),
            "{} missing {part}",
            error.message
        );
    }
    assert!(!error.message.contains("thinking"));
    assert_eq!(result.usage.unwrap().input_tokens, Some(40));
    assert!(result
        .segments
        .iter()
        .any(|s| s.text.as_deref() == Some("temporary failure notice")));
}

#[tokio::test]
async fn valid_tool_response_resets_empty_retry_streak() {
    let model = Model::new(vec![
        Ok(text("")),
        Ok(tool()),
        Ok(text("")),
        Ok(text("final answer")),
    ]);
    let result = run(&model, &mut NoSessionPolicy).await;
    assert_eq!(model.calls.load(Ordering::SeqCst), 4);
    assert_eq!(result.content, "final answer");
    assert_eq!(result.tool_records.len(), 1);
}

#[tokio::test]
async fn recovered_answer_continuation_keeps_facts_but_discards_notice_segments() {
    for synthesis in [false, true] {
        let mut outputs = vec![Ok(tool())];
        if synthesis {
            outputs.push(Err(ModelError::with_kind(
                "unsupported tools",
                ModelErrorKind::ToolsUnsupported,
            )));
        }
        outputs.extend([
            Err(ModelError::with_kind(
                "failure",
                ModelErrorKind::EmptyResponse,
            )),
            Ok(text("final answer")),
        ]);
        let model = Model::new(outputs);
        let result = run(
            &model,
            &mut Recovery {
                continue_after: true,
                ..Default::default()
            },
        )
        .await;
        assert_eq!(result.status, AgentRunStatus::Completed);
        assert_eq!(result.content, "final answer");
        assert_eq!(result.tool_records.len(), 1);
        assert!(result.segments.iter().any(|s| s.kind == ContentKind::Tool));
        assert!(result
            .segments
            .iter()
            .any(|s| s.text.as_deref() == Some("final answer")));
        assert!(!result
            .segments
            .iter()
            .any(|s| s.text.as_deref() == Some("temporary failure notice")));
        assert!(result
            .generated_history
            .iter()
            .any(|m| m.text_content() == "temporary failure notice"));
        assert!(result
            .generated_history
            .iter()
            .any(|m| m.text_content().contains("partial work")));
    }
}

#[tokio::test]
async fn image_only_planning_is_a_valid_answer_without_retry() {
    let mut output = text("");
    output
        .images
        .push(kivio_agent_sdk::model::GeneratedImageData {
            mime_type: "image/png".into(),
            data: "aGVsbG8=".into(),
        });
    let model = Model::new(vec![Ok(output)]);
    let result = run(&model, &mut NoSessionPolicy).await;
    assert_eq!(model.calls.load(Ordering::SeqCst), 1);
    assert_eq!(result.status, AgentRunStatus::Completed);
    assert_eq!(result.images.len(), 1);
}

#[tokio::test]
async fn restoring_recovery_keeps_earlier_valid_text_even_if_it_matches_notice() {
    let model = Model::new(vec![
        Ok(text("temporary failure notice")),
        Ok(tool()),
        Err(ModelError::with_kind(
            "failure",
            ModelErrorKind::EmptyResponse,
        )),
        Ok(text("final answer")),
    ]);
    let result = run(
        &model,
        &mut Recovery {
            continue_after: true,
            ..Default::default()
        },
    )
    .await;
    let notices: Vec<_> = result
        .segments
        .iter()
        .filter(|s| s.text.as_deref() == Some("temporary failure notice"))
        .collect();
    assert_eq!(
        notices.len(),
        1,
        "only the earlier valid planning answer remains"
    );
    assert_eq!(
        notices[0].phase,
        kivio_agent_sdk::agent::content::ContentPhase::Plain
    );
}
