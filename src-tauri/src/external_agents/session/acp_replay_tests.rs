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
    let mut catalog = AcpModelCatalog::default();
    let run = acp_read_prompt(
        &mut reader,
        &mut host_write,
        &mut terminals,
        7,
        "00000000-0000-4000-8000-000000000001",
        false,
        &mut catalog,
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

#[tokio::test]
async fn grok_handshake_model_notifications_supply_usage_only_prompt_capacity() {
    let (host, mut peer) = tokio::io::duplex(8192);
    let (host_read, mut host_write) = tokio::io::split(host);
    let mut reader = BufReader::new(host_read).lines();
    let mut terminals = AcpTerminalHost::new(std::env::temp_dir());
    let (events, mut output) = mpsc::channel(8);
    let (_commands, mut control) = mpsc::channel(1);
    let mut catalog = AcpModelCatalog::default();

    // Grok sends the unscoped catalog and a scoped model_changed before session/new returns its id.
    let advertised = json!({"currentModelId":"launch","availableModels":[
        {"modelId":"launch","_meta":{"totalContextTokens":500000}},
        {"modelId":"native","_meta":{"totalContextTokens":64000}}
    ]});
    let changed = |sid, model| {
        json!({"jsonrpc":"2.0","method":"_x.ai/session_notification",
        "params":{"sessionId":sid,"update":{"sessionUpdate":"model_changed","model_id":model}}})
    };
    let used = json!({"jsonrpc":"2.0","method":"session/update","params":{
        "sessionId":"ours","update":{"sessionUpdate":"usage_update","used":16000}
    }});
    for (reply_id, advertised, model, expected) in [
        (2, advertised, "native", 64_000),
        (
            4,
            json!({"availableModels":[{"modelId":"selected","_meta":{"totalContextTokens":128000}}]}),
            "selected",
            128_000,
        ),
    ] {
        write_rpc_line(
            &mut peer,
            json!({"jsonrpc":"2.0","method":"_x.ai/models/update","params":advertised}),
        )
        .await
        .unwrap();
        write_rpc_line(&mut peer, changed("ours", model))
            .await
            .unwrap();
        write_rpc_line(&mut peer, changed("other", "launch"))
            .await
            .unwrap();
        let (known_session, requested, reply) = if reply_id == 2 {
            (None, None, json!({"sessionId":"ours"}))
        } else {
            (Some("ours"), Some("alias"), json!({}))
        };
        write_rpc_result(&mut peer, &json!(reply_id), reply)
            .await
            .unwrap();
        acp_read_until_id(
            &mut reader,
            &mut host_write,
            Some(&mut terminals),
            catalog.handshake(known_session, requested),
            reply_id,
            DEADLINE,
        )
        .await
        .unwrap()
        .into_result()
        .unwrap();

        write_rpc_line(&mut peer, used.clone()).await.unwrap();
        write_rpc_result(
            &mut peer,
            &json!(reply_id + 1),
            json!({"stopReason":"end_turn"}),
        )
        .await
        .unwrap();
        acp_read_prompt(
            &mut reader,
            &mut host_write,
            &mut terminals,
            reply_id + 1,
            "ours",
            false,
            &mut catalog,
            &events,
            &mut control,
            None,
        )
        .await
        .unwrap();
        assert!(
            matches!(output.try_recv(), Ok(UnifiedAgentEvent::Usage { usage })
            if usage.context_tokens == Some(16000) && usage.context_window_tokens == Some(expected))
        );
    }
}
