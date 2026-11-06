use super::*;
use crate::external_agents::replay_tests::{play, DEADLINE};

#[tokio::test]
async fn offline_replay_adapted_grok_acp_turn_uses_production_reader() {
    let (host, peer) = tokio::io::duplex(65536);
    let (host_read, mut host_write) = tokio::io::split(host);
    let (peer_read, peer_write) = tokio::io::split(peer);
    let mut reader = BufReader::new(host_read).lines();
    let mut terminals = AcpTerminalHost::new(std::env::temp_dir());
    let (events, mut output) = mpsc::channel(128);
    let (_commands, mut control) = mpsc::channel(8);
    let run = acp_read_prompt(
        &mut reader,
        &mut host_write,
        &mut terminals,
        7,
        "00000000-0000-4000-8000-000000000001",
        &events,
        &mut control,
        None,
    );
    let peer = play("acp-adapted-grok-turn", peer_read, peer_write);
    let (result, ()) = timeout(DEADLINE, async { tokio::join!(run, peer) })
        .await
        .unwrap();
    result.unwrap();
    let mut text = String::new();
    let mut thinking = String::new();
    while let Ok(event) = output.try_recv() {
        match event {
            UnifiedAgentEvent::TextDelta { delta } => text.push_str(&delta),
            UnifiedAgentEvent::ThinkingDelta { delta } => thinking.push_str(&delta),
            _ => {}
        }
    }
    assert_eq!(text, "fixture simple ok");
    assert!(!thinking.is_empty());
}
