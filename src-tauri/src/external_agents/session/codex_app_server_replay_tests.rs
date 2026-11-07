use super::*;
use crate::external_agents::replay_tests::{pipe_peer, ready, DEADLINE};

#[tokio::test]
async fn offline_replay_recorded_codex_turn_emits_text_usage_and_delivery() {
    let dir = tempfile::tempdir().unwrap();
    let receipt = dir.path().join("receipt");
    let mut child = pipe_peer("codex-real-turn", &receipt);
    let stdin = child.stdin.take().unwrap();
    let mut reader = BufReader::new(child.stdout.take().unwrap()).lines();
    ready(&mut reader).await;
    let mut session = CodexAppServerSession {
        stdin,
        reader,
        stderr_tail: crate::external_agents::spawn::spawn_stderr_tail(child.stderr.take()),
        child,
        thread_id: "01a0d5eb-f0c0-7481-a417-5e8871805cbc".into(),
        cwd: "/replay".into(),
        cli_bin: std::env::current_exe().unwrap(),
        next_id: 3,
        emitted_tools: HashSet::new(),
        active_turn_id: None,
        seen_turn_ids: HashSet::new(),
        approval_policy: "never",
        sandbox_policy: json!({"type":"dangerFullAccess"}),
        plan_mode: false,
        thread_model: None,
        service_tier: None,
    };
    let delivery = TurnDelivery::default();
    let (events, mut output) = mpsc::channel(64);
    let (_commands, mut control) = mpsc::channel(8);
    timeout(
        DEADLINE,
        session.run_turn(
            &delivery,
            "Respond with the following text: fixture simple ok",
            None,
            None,
            &[],
            &[],
            &events,
            &mut control,
            None,
            None,
        ),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(delivery.possibly_sent());
    let mut text = String::new();
    let mut completed = Vec::new();
    let mut usage = None;
    while let Ok(event) = output.try_recv() {
        match event {
            UnifiedAgentEvent::TextDelta { delta }
            | UnifiedAgentEvent::TextItemDelta { delta, .. } => text.push_str(&delta),
            UnifiedAgentEvent::TextItemCompleted { id, text } => completed.push((id, text)),
            UnifiedAgentEvent::Usage { usage: value } => usage = Some(value),
            _ => {}
        }
    }
    assert_eq!(text, "fixture simple ok");
    assert_eq!(
        completed,
        [(
            "msg_08fda0295ef7a774016ab5bd70c40887d080d27158f71dce50".to_string(),
            "fixture simple ok".to_string()
        )]
    );
    let usage = usage.expect("recorded native usage");
    assert_eq!(usage.total_tokens, Some(22737));
    assert_eq!(usage.output_tokens, Some(7));
    assert!(timeout(DEADLINE, session.child.wait())
        .await
        .unwrap()
        .unwrap()
        .success());
    session.close().await;
    assert_eq!(std::fs::read_to_string(receipt).unwrap(), "complete");
}
