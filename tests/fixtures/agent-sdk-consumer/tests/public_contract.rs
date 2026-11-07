mod support;

use kivio_agent_sdk::{
    agent::{
        content::{AgentToolStatus, ContentKind},
        control::{InputAdmission, StopAdmission},
        events::{AgentEvent, AgentEventKind, AgentRunOutcome, EventDelivery, InputKind},
        history::request_from_history,
        run_agent_loop,
        session::NoSessionPolicy,
        types::AgentRunStatus,
        RuntimeMessage, SteeringMessage,
    },
    model::{LanguageModelProvider, ModelErrorKind, ModelRole},
};
use kivio_agent_sdk_consumer::{config, handle, initial_history, Control, EchoTool, Events, Host};
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Mutex,
};
use support::{answer, event, text_delta, tool, Response, Server, LIMIT};
use tokio::{sync::Notify, time::timeout};

fn assert_lifecycle(events: &Events, terminal: &str) {
    let facts = events.0.lock().unwrap();
    assert_eq!(facts.first().unwrap().kind, "start");
    assert_eq!(facts.last().unwrap().kind, terminal);
    assert_eq!(
        facts
            .iter()
            .filter(|fact| fact.kind.starts_with("end:"))
            .count(),
        1
    );
    for (index, fact) in facts.iter().enumerate() {
        assert_eq!(fact.sequence, index as u64 + 1);
        assert_eq!(fact.invocation, facts[0].invocation);
    }
}

fn messages(body: &Value) -> &[Value] {
    body["messages"].as_array().unwrap()
}

#[tokio::test]
async fn production_http_tool_round_stream_and_history_continuation() {
    let server = Server::start(vec![
        tool("echo", r#"{"text":"hello"}"#),
        answer("done"),
        answer("continued"),
    ])
    .await;
    let provider = server.provider();
    let executor = EchoTool::new(true);
    let control = Control::default();
    let run = handle(&control);
    let events = Events::default();
    let result = timeout(
        LIMIT,
        run_agent_loop(
            config(&provider, initial_history()),
            &Host,
            &executor,
            &mut NoSessionPolicy,
            &events,
            run.clone(),
        ),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(result.status, AgentRunStatus::Completed);
    assert_eq!(result.content, "done");
    assert_eq!(executor.call_count(), 1);
    assert_eq!(server.count(), 2);
    assert_eq!(result.tool_records[0].status, AgentToolStatus::Success);
    assert_eq!(result.tool_records[0].id, "call-42");
    assert!(result
        .generated_history
        .iter()
        .any(|message| message.role() == Some(ModelRole::Tool)));
    assert_lifecycle(&events, "end:Completed");
    let kinds = events
        .0
        .lock()
        .unwrap()
        .iter()
        .map(|fact| fact.kind.clone())
        .collect::<Vec<_>>();
    let index = |kind: &str| kinds.iter().position(|actual| actual == kind).unwrap();
    assert!(index("tool-start:call-42") < index("tool-end:call-42"));
    assert!(index("tool-end:call-42") < index("text:done"));
    assert!(!run.active());
    assert_eq!(run.stop(), StopAdmission::Inactive);
    assert_eq!(
        run.steer(SteeringMessage::new("late".into(), "late").unwrap()),
        InputAdmission::Inactive
    );

    let mut history = result.history;
    history.push(RuntimeMessage::text(ModelRole::User, "continue"));
    let next_control = Control::default();
    let next_events = Events::default();
    let continued = timeout(
        LIMIT,
        run_agent_loop(
            config(&provider, history),
            &Host,
            &executor,
            &mut NoSessionPolicy,
            &next_events,
            handle(&next_control),
        ),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(continued.content, "continued");
    assert_eq!(executor.call_count(), 1);
    assert_eq!(server.count(), 3);
    assert_lifecycle(&next_events, "end:Completed");
    let requests = server.requests.lock().unwrap();
    assert!(requests[0]
        .headers
        .to_lowercase()
        .contains("authorization: bearer fixture-key"));
    assert!(requests[0]
        .headers
        .to_lowercase()
        .contains("x-consumer: standalone"));
    assert_eq!(requests[0].body["stream"], true);
    for request in &requests[1..] {
        let history = messages(&request.body);
        let call = history
            .iter()
            .find(|message| message["tool_calls"].is_array())
            .unwrap();
        assert_eq!(call["tool_calls"][0]["id"], "call-42");
        let output = history
            .iter()
            .find(|message| message["role"] == "tool")
            .unwrap();
        assert_eq!(output["tool_call_id"], "call-42");
        assert_eq!(output["content"], "hello");
    }
    assert_eq!(
        messages(&requests[2].body).last().unwrap()["content"],
        "continue"
    );
    let replay = messages(&requests[2].body);
    assert_eq!(replay[replay.len() - 2]["content"], "done");
}

#[tokio::test]
async fn inputs_are_consumed_once_at_tool_and_answer_boundaries() {
    let server = Server::start(vec![
        tool("echo", r#"{"text":"hello"}"#),
        answer("first answer"),
        answer("followed"),
    ])
    .await;
    let provider = server.provider();
    let control = Control::default();
    let run = handle(&control);
    let events = Events::default();
    let queued = AtomicBool::new(false);
    let sink = |event: &AgentEvent<'_>| {
        use kivio_agent_sdk::agent::events::AgentEventSink;
        events.on_event(event);
        if matches!(event.kind, AgentEventKind::ToolEnded { .. })
            && !queued.swap(true, Ordering::SeqCst)
        {
            assert_eq!(
                run.steer(SteeringMessage::new("steer".into(), "steer now").unwrap()),
                InputAdmission::Accepted
            );
            assert_eq!(
                run.follow_up(SteeringMessage::new("follow".into(), "ask next").unwrap()),
                InputAdmission::Accepted
            );
        }
        EventDelivery::Accepted
    };
    let executor = EchoTool::new(true);
    let result = timeout(
        LIMIT,
        run_agent_loop(
            config(&provider, initial_history()),
            &Host,
            &executor,
            &mut NoSessionPolicy,
            &sink,
            run.clone(),
        ),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(result.status, AgentRunStatus::Completed);
    assert_eq!(executor.call_count(), 1);
    assert_eq!(server.count(), 3);
    assert_lifecycle(&events, "end:Completed");
    let inputs = result
        .segments
        .iter()
        .filter_map(|block| match &block.kind {
            ContentKind::Input { kind, message } => Some((*kind, message.id.as_str())),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        inputs,
        vec![(InputKind::Steer, "steer"), (InputKind::FollowUp, "follow")]
    );
    let requests = server.requests.lock().unwrap();
    assert_eq!(
        messages(&requests[1].body).last().unwrap()["content"],
        "steer now"
    );
    assert!(!messages(&requests[1].body)
        .iter()
        .any(|message| message["content"] == "ask next"));
    let last = messages(&requests[2].body);
    assert_eq!(last.last().unwrap()["content"], "ask next");
    assert_eq!(last[last.len() - 2]["content"], "first answer");
    for text in ["steer now", "ask next"] {
        assert_eq!(
            last.iter()
                .filter(|message| message["content"] == text)
                .count(),
            1
        );
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
}

#[tokio::test]
async fn tool_free_run_consumes_steer_and_follow_up_once() {
    let server = Server::start(vec![answer("first answer"), answer("followed up")]).await;
    let provider = server.provider();
    let control = Control::default();
    let run = handle(&control);
    assert_eq!(
        run.steer(SteeringMessage::new("steer".into(), "steer now").unwrap()),
        InputAdmission::Accepted
    );
    assert_eq!(
        run.follow_up(SteeringMessage::new("follow".into(), "ask next").unwrap()),
        InputAdmission::Accepted
    );
    let mut config = config(&provider, initial_history());
    config.tools.clear();
    let executor = EchoTool::new(true);
    let events = Events::default();
    let result = timeout(
        LIMIT,
        run_agent_loop(config, &Host, &executor, &mut NoSessionPolicy, &events, run),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(result.status, AgentRunStatus::Completed);
    assert_eq!(result.content, "followed up");
    let inputs = result
        .segments
        .iter()
        .filter_map(|block| match &block.kind {
            ContentKind::Input { kind, message } => Some((*kind, message.id.as_str())),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        inputs,
        vec![(InputKind::Steer, "steer"), (InputKind::FollowUp, "follow")]
    );
    for text in ["first answer", "followed up"] {
        assert_eq!(
            result
                .segments
                .iter()
                .filter(|block| block.text.as_deref() == Some(text))
                .count(),
            1
        );
    }

    assert_eq!(server.count(), 2);
    assert_eq!(executor.call_count(), 0);
    assert_lifecycle(&events, "end:Completed");
    let requests = server.requests.lock().unwrap();
    assert_eq!(
        messages(&requests[0].body).last().unwrap()["content"],
        "steer now"
    );
    assert!(!messages(&requests[0].body)
        .iter()
        .any(|message| message["content"] == "ask next"));
    let last = messages(&requests[1].body);
    assert_eq!(last.last().unwrap()["content"], "ask next");
    assert_eq!(last[last.len() - 2]["content"], "first answer");
    for text in ["steer now", "ask next", "first answer"] {
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
            1
        );
    }
}

#[tokio::test]
async fn unsupported_tools_fallback_stream_has_an_open_message() {
    let server = Server::start(vec![
        Response::Http(
            400,
            r#"{"error":{"message":"tools not supported","type":"invalid_request_error"}}"#.into(),
        ),
        answer("plain fallback"),
    ])
    .await;
    let provider = server.provider();
    let control = Control::default();
    let run = handle(&control);
    let open = AtomicBool::new(false);
    let outside_message = AtomicBool::new(false);
    let step = AtomicUsize::new(0);
    let rounds = Mutex::new(Vec::new());
    let events = Events::default();
    let sink = |event: &AgentEvent<'_>| {
        use kivio_agent_sdk::agent::events::AgentEventSink;
        events.on_event(event);
        match &event.kind {
            AgentEventKind::StepStarted { round } => {
                assert_eq!(step.swap(*round as usize, Ordering::SeqCst), 0);
                rounds.lock().unwrap().push(*round);
            }
            AgentEventKind::StepEnded { round } => {
                assert!(!open.load(Ordering::SeqCst));
                assert_eq!(step.swap(0, Ordering::SeqCst), *round as usize);
            }
            AgentEventKind::MessageStarted { round } => {
                assert_eq!(step.load(Ordering::SeqCst), *round as usize);
                assert!(!open.swap(true, Ordering::SeqCst));
            }
            AgentEventKind::MessageEnded { .. } => {
                assert!(open.swap(false, Ordering::SeqCst));
            }
            AgentEventKind::Content { delta, .. } if !delta.is_empty() => {
                if !open.load(Ordering::SeqCst) || step.load(Ordering::SeqCst) == 0 {
                    outside_message.store(true, Ordering::SeqCst);
                }
            }
            _ => {}
        }
        EventDelivery::Accepted
    };
    let executor = EchoTool::new(true);
    let result = timeout(
        LIMIT,
        run_agent_loop(
            config(&provider, initial_history()),
            &Host,
            &executor,
            &mut NoSessionPolicy,
            &sink,
            run,
        ),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(result.status, AgentRunStatus::Completed);
    assert_eq!(result.content, "plain fallback");
    assert_eq!(server.count(), 2);
    assert_eq!(executor.call_count(), 0);
    assert!(
        !outside_message.load(Ordering::SeqCst),
        "fallback content must be inside a model message"
    );
    assert!(!open.load(Ordering::SeqCst));
    assert_eq!(step.load(Ordering::SeqCst), 0);
    assert_eq!(*rounds.lock().unwrap(), vec![1, 2]);
    assert_lifecycle(&events, "end:Completed");
}

#[tokio::test]
async fn cancellation_drops_real_http_stream_and_emits_one_terminal() {
    for use_tools in [false, true] {
        let server = Server::start(vec![Response::Hold(text_delta("partial answer\n"))]).await;
        let provider = server.provider();
        let control = Control::default();
        let run = handle(&control);
        let events = Events::default();
        let received = Notify::new();
        let sink = |event: &AgentEvent<'_>| {
            use kivio_agent_sdk::agent::events::AgentEventSink;
            events.on_event(event);
            if matches!(
                event.kind,
                AgentEventKind::Content {
                    delta: "partial answer\n",
                    ..
                }
            ) {
                received.notify_one();
            }
            EventDelivery::Accepted
        };
        let executor = EchoTool::new(true);
        let mut policy = NoSessionPolicy;
        let mut cfg = config(&provider, initial_history());
        if !use_tools {
            cfg.tools.clear();
        }
        let execution = run_agent_loop(cfg, &Host, &executor, &mut policy, &sink, run.clone());
        let stopping = async {
            received.notified().await;
            assert_eq!(run.stop(), StopAdmission::Requested);
            run.wait_inactive().await;
        };
        let (result, ()) = timeout(LIMIT, async { tokio::join!(execution, stopping) })
            .await
            .unwrap();
        let result = result.unwrap();
        assert_eq!(result.status, AgentRunStatus::Cancelled);
        assert_eq!(result.content, "partial answer");
        timeout(LIMIT, server.disconnected.notified())
            .await
            .unwrap();
        assert_eq!(executor.call_count(), 0);
        assert_eq!(server.count(), 1);
        assert_lifecycle(&events, &format!("end:{:?}", AgentRunOutcome::Cancelled));
        assert_eq!(
            run.follow_up(SteeringMessage::new("late".into(), "late").unwrap()),
            InputAdmission::Inactive
        );
    }
}

#[tokio::test]
async fn host_denial_unknown_tool_and_bad_arguments_have_no_side_effects() {
    for (name, arguments, allowed, expected) in [
        ("echo", r#"{"text":42}"#, true, "must be string"),
        ("echo", r#"{"text":"hello"}"#, false, "Host denied echo"),
        ("echo", "{broken", true, "invalid or incomplete"),
        ("unregistered", "{}", true, "unknown"),
    ] {
        let server = Server::start(vec![tool(name, arguments), answer("rejected safely")]).await;
        let provider = server.provider();
        let executor = EchoTool::new(allowed);
        let control = Control::default();
        let events = Events::default();
        let result = timeout(
            LIMIT,
            run_agent_loop(
                config(&provider, initial_history()),
                &Host,
                &executor,
                &mut NoSessionPolicy,
                &events,
                handle(&control),
            ),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(executor.call_count(), 0);
        assert_eq!(server.count(), 2);
        assert_eq!(result.tool_records[0].status, AgentToolStatus::Error);
        assert!(events
            .0
            .lock()
            .unwrap()
            .iter()
            .any(|event| event.kind == "tool-update:Error"));
        let diagnostic = result.tool_records[0].error.as_ref().unwrap();
        assert!(
            diagnostic.to_lowercase().contains(&expected.to_lowercase()),
            "{diagnostic}"
        );
        assert_lifecycle(&events, "end:Completed");
    }
}

#[tokio::test]
async fn wire_errors_remain_typed_and_do_not_retry() {
    let cases = [
        (
            Response::Http(
                401,
                json!({"error":{"code":"invalid_api_key","message":"denied"}}).to_string(),
            ),
            ModelErrorKind::Auth,
            Some(401),
            false,
        ),
        (
            Response::Http(
                429,
                json!({"error":{"code":"rate_limit_exceeded","message":"busy"}}).to_string(),
            ),
            ModelErrorKind::RateLimited,
            Some(429),
            false,
        ),
        (
            Response::Disconnect(text_delta("unfinished")),
            ModelErrorKind::Transport,
            None,
            true,
        ),
        (
            Response::Sse(text_delta("missing terminal")),
            ModelErrorKind::Transport,
            None,
            true,
        ),
        (
            Response::Sse("data: {malformed\n\n".into()),
            ModelErrorKind::Transport,
            None,
            true,
        ),
    ];
    for (response, kind, status, retryable) in cases {
        let server = Server::start(vec![response]).await;
        let provider = server.provider();
        let request = request_from_history(
            &initial_history(),
            "test-model",
            vec![],
            Default::default(),
            Default::default(),
        );
        let error = timeout(LIMIT, provider.stream(request, &mut |_| Ok(())))
            .await
            .unwrap()
            .unwrap_err();
        assert_eq!(error.kind, kind);
        assert_eq!(error.status, status);
        assert_eq!(error.is_retryable_stream_error(), retryable);
        assert_eq!(server.count(), 1);
        if status == Some(401) {
            assert_eq!(error.provider_code.as_deref(), Some("invalid_api_key"));
        }
    }
}

#[tokio::test]
async fn finish_reason_commits_before_a_broken_tail_without_retry() {
    let body = format!(
        "{}{}",
        text_delta("committed"),
        event(json!({"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}))
    );
    let server = Server::start(vec![Response::Disconnect(body)]).await;
    let provider = server.provider();
    let control = Control::default();
    let events = Events::default();
    let result = timeout(
        LIMIT,
        run_agent_loop(
            config(&provider, initial_history()),
            &Host,
            &EchoTool::new(true),
            &mut NoSessionPolicy,
            &events,
            handle(&control),
        ),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(result.status, AgentRunStatus::Completed);
    assert_eq!(result.content, "committed");
    assert_eq!(server.count(), 1);
    assert_lifecycle(&events, "end:Completed");
}

#[tokio::test]
async fn cancelled_follow_up_stream_preserves_prior_answer_and_current_reasoning() {
    for partial_kind in ["empty", "text", "reasoning"] {
        let Response::Sse(first) = answer("first answer") else {
            unreachable!()
        };
        let reasoning =
            |text: &str| event(json!({"choices":[{"index":0,"delta":{"reasoning_content":text}}]}));
        let first = format!("{}{}", reasoning("first thought"), first);
        let partial = match partial_kind {
            "empty" => String::new(),
            "text" => text_delta("partial answer\n"),
            "reasoning" => reasoning("second thought"),
            _ => unreachable!(),
        };
        let server = Server::start(vec![Response::Sse(first), Response::Hold(partial)]).await;
        let provider = server.provider();
        let control = Control::default();
        let run = handle(&control);
        run.follow_up(SteeringMessage::new("follow".into(), "ask next").unwrap());
        let mut cfg = config(&provider, initial_history());
        cfg.tools.clear();
        let events = Events::default();
        let received = Notify::new();
        let sink = |event: &AgentEvent<'_>| {
            use kivio_agent_sdk::agent::events::AgentEventSink;
            events.on_event(event);
            if matches!(&event.kind, AgentEventKind::Content { delta, reasoning_delta, .. } if *delta == "partial answer\n" || *reasoning_delta == Some("second thought"))
            {
                received.notify_one();
            }
            EventDelivery::Accepted
        };
        let executor = EchoTool::new(true);
        let mut policy = NoSessionPolicy;
        let execution = run_agent_loop(cfg, &Host, &executor, &mut policy, &sink, run.clone());
        let stopping = async {
            if partial_kind == "empty" {
                while server.count() < 2 {
                    tokio::time::sleep(std::time::Duration::from_millis(1)).await;
                }
            } else {
                received.notified().await;
            }
            assert_eq!(run.stop(), StopAdmission::Requested);
        };
        let (result, ()) = timeout(LIMIT, async { tokio::join!(execution, stopping) })
            .await
            .unwrap();
        let result = result.expect("cancellation must retain the completed first answer");
        assert_eq!(result.status, AgentRunStatus::Cancelled);
        assert_eq!(server.count(), 2);
        assert_eq!(executor.call_count(), 0);
        for history in [result.history.context(), &result.generated_history] {
            assert_eq!(
                history
                    .iter()
                    .filter(|message| message.text_content() == "first answer")
                    .count(),
                1
            );
            assert_eq!(
                history
                    .iter()
                    .filter(|message| message.text_content() == "ask next")
                    .count(),
                1
            );
            let assistants = history
                .iter()
                .filter_map(|message| message.model_message())
                .filter(|message| message.role == ModelRole::Assistant)
                .collect::<Vec<_>>();
            assert_eq!(
                assistants.len(),
                if partial_kind == "empty" { 1 } else { 2 }
            );
            if partial_kind != "empty" {
                let reasoning = assistants[1]
                    .content
                    .iter()
                    .filter_map(|part| match part {
                        kivio_agent_sdk::model::MessagePart::Reasoning { text } => {
                            Some(text.as_str())
                        }
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                assert_eq!(
                    reasoning,
                    if partial_kind == "reasoning" {
                        vec!["second thought"]
                    } else {
                        vec![]
                    }
                );
            }
        }
        assert_eq!(
            result.reasoning.as_deref(),
            Some(if partial_kind == "reasoning" {
                "first thought\n\nsecond thought"
            } else {
                "first thought"
            })
        );
        assert_eq!(
            result
                .segments
                .iter()
                .filter(|block| block.text.as_deref() == Some("first answer"))
                .count(),
            1
        );
        if partial_kind == "reasoning" {
            assert_eq!(
                result
                    .segments
                    .iter()
                    .filter(|block| block.kind == ContentKind::Reasoning
                        && block.text.as_deref() == Some("second thought"))
                    .count(),
                1
            );
        }
        assert_lifecycle(&events, "end:Cancelled");
        timeout(LIMIT, server.disconnected.notified())
            .await
            .unwrap();
    }
}
