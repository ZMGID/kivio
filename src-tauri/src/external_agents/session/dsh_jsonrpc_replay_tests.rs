use super::*;
use crate::external_agents::replay_tests::{pipe_peer, ready, DEADLINE};

#[tokio::test]
async fn offline_replay_dsh_two_prompts_and_disconnect_keep_native_delivery() {
    for (fixture, prompts) in [
        ("dsh-two-turns", vec!["first", "second"]),
        ("dsh-disconnect", vec!["probe"]),
        ("dsh-stop-next", vec!["first", "second"]),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let receipt = dir.path().join("receipt");
        let mut child = pipe_peer(fixture, &receipt);
        let stdin = child.stdin.take().unwrap();
        let mut reader = BufReader::new(child.stdout.take().unwrap()).lines();
        ready(&mut reader).await;
        let mut session = DshJsonRpcSession {
            stdin,
            reader,
            stderr_tail: crate::external_agents::spawn::spawn_stderr_tail(child.stderr.take()),
            child,
            session_id: "replay-session".into(),
            resumed: false,
            next_id: 1,
            route: resolve_model_route_for_turn(None).unwrap(),
            context_window: None,
            map_state: MapSessionState::default(),
            child_progress: HashMap::new(),
            idle_wake: IdleWakeState::default(),
            slash_commands: Vec::new(),
        };
        let (events, mut output) = mpsc::channel(64);
        let (commands, mut control) = mpsc::channel(8);
        for prompt in prompts {
            let delivery = TurnDelivery::default();
            let mut pending = HashMap::new();
            let run = session.run_turn(
                &delivery,
                prompt,
                None,
                &[],
                &events,
                &mut control,
                None,
                None,
                &mut pending,
            );
            let cancelled = fixture == "dsh-stop-next" && prompt == "first";
            let result = if cancelled {
                let stop = async {
                    while let Some(event) = output.recv().await {
                        if matches!(event, UnifiedAgentEvent::TextDelta { delta } if delta == "before-stop")
                        {
                            commands.send(SessionCommand::Cancel).await.unwrap();
                            return;
                        }
                    }
                    panic!("missing active-turn output before stop");
                };
                let (result, ()) = timeout(DEADLINE, async { tokio::join!(run, stop) })
                    .await
                    .unwrap();
                result
            } else {
                timeout(DEADLINE, run).await.unwrap()
            };
            assert!(delivery.possibly_sent());
            let disconnected = fixture == "dsh-disconnect";
            if cancelled {
                assert_eq!(result.unwrap_err(), "cancelled");
            } else {
                assert_eq!(result.is_err(), disconnected, "{result:?}");
            }
            let mut text = String::new();
            while let Ok(event) = output.try_recv() {
                if let UnifiedAgentEvent::TextDelta { delta } = event {
                    text.push_str(&delta);
                }
            }
            assert_eq!(
                text,
                if disconnected || cancelled {
                    ""
                } else {
                    prompt
                }
            );
        }
        assert!(timeout(DEADLINE, session.child.wait())
            .await
            .unwrap()
            .unwrap()
            .success());
        session.close().await;
        assert_eq!(std::fs::read_to_string(receipt).unwrap(), "complete");
    }
}
