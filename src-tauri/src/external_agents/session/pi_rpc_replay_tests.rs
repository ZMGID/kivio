use super::*;
use crate::external_agents::replay_tests::{pipe_peer, ready, DEADLINE};

#[tokio::test]
async fn offline_replay_recorded_pi_turn_emits_text_usage_and_delivery() {
    use crate::external_agents::diagnostics::NativeDiagnostics;
    use crate::external_agents::session::live::{SessionCommand, TurnDelivery};
    use crate::external_agents::types::StreamFormat;

    let dir = tempfile::tempdir().unwrap();
    let receipt = dir.path().join("receipt");
    let (_diagnostics, capture, trace) = NativeDiagnostics::test_capture(StreamFormat::PiRpc);
    let actor = capture
        .scope_spawn(async {
            let mut child = pipe_peer("pi-real-turn", &receipt);
            let stdin = Arc::new(Mutex::new(child.stdin.take().unwrap()));
            let mut reader = BufReader::new(child.stdout.take().unwrap()).lines();
            ready(&mut reader).await;
            let stderr = crate::external_agents::spawn::spawn_stderr_tail(child.stderr.take());
            spawn_pi_rpc_session_actor(PiRpcSession {
                _mcp_extension: None,
                child,
                stdin,
                reader: Arc::new(Mutex::new(reader)),
                session_id: "replay-session".into(),
                resumed: false,
                selection: PiSelection::default(),
                stderr_tail: Arc::new(Mutex::new(String::new())),
                _stderr_task: tokio::spawn(async move {
                    let _ = stderr.await;
                }),
            })
        })
        .await;
    let _generation = actor
        .observe_generation(17)
        .expect("captured actor connection");
    let delivery = TurnDelivery::default();
    let (events, mut output) = mpsc::channel(64);
    let (done, completion) = oneshot::channel();
    actor
        .send(SessionCommand::RunTurn {
            delivery: delivery.clone(),
            cancelled: None,
            prompt: "Respond with the following text: fixture simple ok".into(),
            model: None,
            reasoning: None,
            images: vec![],
            extra_writable_roots: vec![],
            events,
            done,
            approvals: None,
        })
        .await
        .unwrap();
    timeout(DEADLINE, completion)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(delivery.possibly_sent());
    let mut events = Vec::new();
    while let Ok(event) = output.try_recv() {
        events.push(event);
    }
    let text: String = events
        .iter()
        .filter_map(|event| match event {
            UnifiedAgentEvent::TextDelta { delta } => Some(delta.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(text, "fixture simple ok");
    assert!(events.iter().any(|event| matches!(event, UnifiedAgentEvent::Usage { usage } if usage.total_tokens == Some(1814) && usage.output_tokens == Some(28))));
    timeout(DEADLINE, actor.finished()).await.unwrap();
    assert_eq!(std::fs::read_to_string(receipt).unwrap(), "complete");
    let records: Vec<Value> = trace
        .try_iter()
        .map(|(_, line)| serde_json::from_slice(&line).unwrap())
        .collect();
    let frames: Vec<_> = records
        .iter()
        .filter(|record| record["event"] == "frame")
        .collect();
    for kind in ["prompt", "message_update", "agent_end"] {
        assert!(
            frames
                .iter()
                .any(|record| record["kind"] == kind && record["observed_during_generation"] == 17),
            "missing Pi actor trace for {kind}"
        );
    }
    let connection = frames[0]["connection"].clone();
    assert!(frames
        .iter()
        .all(|record| record["connection"] == connection));
    let serialized = serde_json::to_string(&records).unwrap();
    assert!(!serialized.contains("fixture simple ok"));
    assert!(!serialized.contains("The user wants me"));
}
