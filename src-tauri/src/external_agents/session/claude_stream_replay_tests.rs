use super::*;
use crate::external_agents::replay_tests::{pipe_peer, ready, DEADLINE};

#[tokio::test]
async fn offline_replay_claude_recorded_approval_fails_closed_and_disconnect_keeps_delivery_uncertain(
) {
    for (fixture, disconnected) in [
        ("claude-recorded-approval", false),
        ("claude-disconnect", true),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let receipt = dir.path().join("receipt");
        let mut child = pipe_peer(fixture, &receipt);
        let stdin = child.stdin.take().unwrap();
        let mut reader = BufReader::new(child.stdout.take().unwrap()).lines();
        ready(&mut reader).await;
        let mut session = ClaudeStreamJsonSession {
            stdin,
            reader,
            stderr_tail: Some(crate::external_agents::spawn::spawn_stderr_tail(
                child.stderr.take(),
            )),
            child,
            chat_mcp_config: None,
            handler: create_stream_handler(StreamFormat::ClaudeStreamJson),
            session_id: String::new(),
            active_model: None,
            stale_frames_left: 0,
            idle_handler: None,
            idle_wake_text: String::new(),
            idle_wake_usage: None,
        };
        let (events, mut output) = mpsc::channel(64);
        let (_commands, mut control) = mpsc::channel(8);
        let delivery = TurnDelivery::default();
        let result = timeout(
            DEADLINE,
            session.run_turn(
                &delivery,
                "probe",
                None,
                &[],
                &events,
                &mut control,
                None,
                None,
            ),
        )
        .await
        .unwrap();
        assert!(delivery.possibly_sent());
        assert_eq!(result.is_err(), disconnected, "{result:?}");
        let mut text = String::new();
        while let Ok(event) = output.try_recv() {
            if let UnifiedAgentEvent::TextDelta { delta } = event {
                text.push_str(&delta);
            }
        }
        assert_eq!(
            text,
            if disconnected {
                ""
            } else {
                "permission unavailable"
            }
        );
        assert!(timeout(DEADLINE, session.child.wait())
            .await
            .unwrap()
            .unwrap()
            .success());
        session.close().await;
        assert_eq!(std::fs::read_to_string(receipt).unwrap(), "complete");
    }
}
