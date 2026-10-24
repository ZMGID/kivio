//! Real UI-service → existing CLI protocol → pipes/process/store boundary.
#![cfg(unix)]
use kivio_model_runtime::{conversation_data::Conversation, conversation_files::ConversationStore};
use serde_json::{Value, json};
use std::{os::unix::fs::PermissionsExt, path::Path, time::Duration};
use zeron_ui::{EngineBootConfig, HarnessId, state::EngineHandle};

async fn finished(store: &ConversationStore, id: &str, count: usize) -> Conversation {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if let Ok(c) = store.load(id) {
                if c.messages.iter().filter(|m| m.role == "assistant").count() >= count {
                    return c;
                }
            }
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
    })
    .await
    .expect("CLI reply did not finish")
}
fn boot(root: &Path) -> EngineBootConfig {
    EngineBootConfig {
        data_dir: root.into(),
        ipc_port: 0,
        edge_url: String::new(),
        edge_token: None,
        org_id: None,
        workos_client_id: None,
        default_harness: HarnessId::Codex,
    }
}
async fn deny_next_question(engine: &EngineHandle, chat: &str) {
    let mut updates = engine
        .client()
        .subscribe("WatchDocMessages", json!({"chatId":chat}))
        .await
        .unwrap();
    let (request_id, question_id) = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let frame = updates.recv().await.unwrap();
            for entry in frame["reset"].as_array().unwrap() {
                for part in entry["parts"].as_array().unwrap() {
                    if part["kind"] == "input" && part["resolved"] == false {
                        return (
                            part["requestId"].as_str().unwrap().to_owned(),
                            part["questions"][0]["id"].as_str().unwrap().to_owned(),
                        );
                    }
                }
            }
        }
    })
    .await
    .unwrap();
    engine.client().call("QueueCommand", json!({"chatId":chat,"command":{"kind":"respondInput","requestId":request_id,"answers":[{"questionId":question_id,"labels":["拒绝"]}]}})).await.unwrap();
}
#[tokio::test]
async fn cli_sends_tools_and_attachments_resumes_and_cancels_without_api_provider() {
    let dir = tempfile::tempdir().unwrap();
    let executable = dir.path().join("codex-peer");
    std::fs::write(&executable, include_str!("fixtures/codex_peer.py")).unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
    let log = dir.path().join("wire.jsonl");
    std::fs::write(dir.path().join("settings.json"), serde_json::to_vec(&json!({"settings":{"chat":{"externalCliAgents":{"codex":{"path":executable,"env":[{"key":"KIVIO_CLI_TEST_LOG","value":log}]}}}}})).unwrap()).unwrap();
    let store = ConversationStore::new(dir.path().join("conversations"));
    let engine = EngineHandle::bootstrap(boot(dir.path())).await.unwrap();
    engine.client().call("KivioSend", json!({"chatId":"conv_cli","runtime":"codex","text":"read this","attachments":[{"name":"attachment.txt","data":"aGVsbG8="}]})).await.unwrap();
    deny_next_question(&engine, "conv_cli").await;
    let first = finished(&store, "conv_cli", 1).await;
    assert_eq!(first.messages[1].content, "final corrected answer");
    assert_eq!(
        first.messages[1].stream_outcome.as_deref(),
        Some("completed")
    );
    assert_eq!(
        first.messages[1].tool_calls[0].result_preview.as_deref(),
        Some("hello")
    );
    engine.shutdown().await;
    let engine = EngineHandle::bootstrap(boot(dir.path())).await.unwrap();
    // The saved runtime wins even when a client mistakenly asks for builtin.
    engine
        .client()
        .call(
            "KivioSend",
            json!({"chatId":"conv_cli","runtime":"builtin","text":"continue"}),
        )
        .await
        .unwrap();
    deny_next_question(&engine, "conv_cli").await;
    finished(&store, "conv_cli", 2).await;
    let lines = std::fs::read_to_string(&log).unwrap();
    let wire: Vec<Value> = lines
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(
        wire.iter()
            .filter(|v| v["request"]["method"] == "thread/start")
            .count(),
        1
    );
    assert!(
        wire.iter()
            .any(|v| v["request"]["method"] == "thread/resume"
                && v["request"]["params"]["threadId"] == "native-thread")
    );
    assert!(
        wire.iter().any(|v| v["request"]["id"] == "approval-1"
            && v["request"]["result"]["decision"] == "decline"),
        "the UI answer must reach the original CLI approval response"
    );
    let first_turn = wire
        .iter()
        .find(|v| v["request"]["method"] == "turn/start")
        .unwrap();
    let prompt = first_turn["request"]["params"]["input"][0]["text"]
        .as_str()
        .unwrap();
    assert!(prompt.contains("Attached file:") && prompt.contains("conv_cli_attachments"));
    engine
        .client()
        .call(
            "KivioSend",
            json!({"chatId":"conv_cli","text":"WAIT_FOR_CANCEL"}),
        )
        .await
        .unwrap();
    // Wait for the child to actually accept this turn, not merely for the host to register a run.
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if std::fs::read_to_string(&log)
                .unwrap()
                .contains("WAIT_FOR_CANCEL")
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    engine
        .client()
        .call("KivioCancel", json!({"chatId":"conv_cli"}))
        .await
        .unwrap();
    let last = finished(&store, "conv_cli", 3).await;
    assert_eq!(
        last.messages.last().unwrap().stream_outcome.as_deref(),
        Some("cancelled")
    );
    engine.shutdown().await;
    let wire = std::fs::read_to_string(&log).unwrap();
    assert!(
        wire.contains("turn/interrupt"),
        "cancel must interrupt the native turn"
    );
    let pid = serde_json::from_str::<Value>(wire.lines().last().unwrap()).unwrap()["pid"]
        .as_u64()
        .unwrap();
    assert!(
        !std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap()
            .success(),
        "CLI process survived shutdown"
    );
    assert!(
        engine
            .client()
            .call(
                "KivioSend",
                json!({"chatId":"conv_unsupported","runtime":"does-not-exist","text":"hello"})
            )
            .await
            .is_err()
    );
    assert!(store.load("conv_unsupported").is_err());
}

#[tokio::test]
async fn imported_drivers_use_kivio_config_questions_and_durable_resume() {
    for (agent, fixture, answer, resume_method) in [
        (
            "grok",
            include_str!("fixtures/acp_peer.py"),
            "ACP reply",
            "session/load",
        ),
        (
            "claude",
            include_str!("fixtures/claude_peer.py"),
            "Claude reply",
            "--resume=claude-session",
        ),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let executable = dir.path().join("acp-peer");
        std::fs::write(&executable, fixture).unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        let log = dir.path().join("acp.jsonl");
        std::fs::write(dir.path().join("settings.json"), serde_json::to_vec(&json!({"settings":{"chat":{"externalCliAgents":{(agent):{"path":executable,"env":[{"key":"KIVIO_CLI_TEST_LOG","value":log}]}}}}})).unwrap()).unwrap();
        let store = ConversationStore::new(dir.path().join("conversations"));
        for count in 1..=2 {
            let engine = EngineHandle::bootstrap(boot(dir.path())).await.unwrap();
            engine
                .client()
                .call(
                    "KivioSend",
                    json!({"chatId":"conv_acp","runtime":agent,"text":"hello"}),
                )
                .await
                .unwrap();
            deny_next_question(&engine, "conv_acp").await;
            let c = finished(&store, "conv_acp", count).await;
            assert_eq!(c.messages.last().unwrap().content, answer);
            assert_eq!(
                c.messages.last().unwrap().stream_outcome.as_deref(),
                Some("completed")
            );
            engine.shutdown().await;
        }
        if agent == "grok" {
            std::fs::write(format!("{}.reject-resume", log.display()), "").unwrap();
            let engine = EngineHandle::bootstrap(boot(dir.path())).await.unwrap();
            engine
                .client()
                .call("KivioSend", json!({"chatId":"conv_acp","text":"continue"}))
                .await
                .unwrap();
            let failed = finished(&store, "conv_acp", 3).await;
            assert_eq!(
                failed.messages.last().unwrap().stream_outcome.as_deref(),
                Some("error")
            );
            assert!(
                failed
                    .messages
                    .last()
                    .unwrap()
                    .content
                    .contains("session missing")
            );
            engine.shutdown().await;
        }
        let wire: Vec<Value> = std::fs::read_to_string(log)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        if agent == "grok" {
            assert_eq!(
                wire.iter()
                    .filter(|v| v["request"]["method"] == "session/new")
                    .count(),
                1
            );
            assert_eq!(
                wire.iter()
                    .filter(|v| v["request"]["method"] == resume_method)
                    .count(),
                2
            );
        } else {
            assert!(wire.iter().any(|v| {
                v["args"]
                    .as_array()
                    .is_some_and(|a| a.iter().any(|arg| arg == resume_method))
            }));
        }
        for pid in wire.iter().filter_map(|v| v["pid"].as_u64()) {
            assert!(
                !std::process::Command::new("kill")
                    .args(["-0", &pid.to_string()])
                    .stderr(std::process::Stdio::null())
                    .status()
                    .unwrap()
                    .success()
            );
        }
    }
}
