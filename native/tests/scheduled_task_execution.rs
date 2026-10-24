//! Real native service, local HTTP inference and durable task/conversation owners.
use kivio_model_runtime::{
    conversation_files::ConversationStore,
    scheduled_tasks::{self, types::*},
};
use serde_json::json;
use std::{
    io::{Read, Write},
    net::TcpListener,
    time::Duration,
};
use zeron_ui::{EngineBootConfig, HarnessId, state::EngineHandle};

async fn wait(mut ready: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(15), async {
        while !ready() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("native task did not reach its final state");
}
fn request(stream: &mut std::net::TcpStream) -> Option<String> {
    stream
        .set_read_timeout(Some(Duration::from_secs(15)))
        .unwrap();
    let mut bytes = vec![];
    let mut buf = [0; 4096];
    loop {
        let n = stream.read(&mut buf).unwrap();
        assert_ne!(n, 0);
        bytes.extend_from_slice(&buf[..n]);
        if let Some(end) = bytes.windows(4).position(|s| s == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&bytes[..end]).to_lowercase();
            let len = head
                .lines()
                .find_map(|s| {
                    s.strip_prefix("content-length:")
                        .map(|s| s.trim().parse::<usize>().unwrap())
                })
                .unwrap_or(0);
            if bytes.len() >= end + 4 + len {
                if !head.starts_with("post /v1/chat/completions ") {
                    return None;
                }
                return Some(String::from_utf8(bytes[end + 4..end + 4 + len].to_vec()).unwrap());
            }
        }
    }
}
fn draft() -> ScheduledTaskInput {
    ScheduledTaskInput {
        id: None,
        name: "Native schedule".into(),
        prompt: "old scheduled prompt".into(),
        schedule: ScheduleRule::Interval {
            minutes: 60,
            anchor_at: None,
        },
        target: TaskTarget::NewConversation {
            provider_id: None,
            model: None,
            project_id: None,
            thinking_level: None,
        },
        enabled: Some(true),
    }
}

#[tokio::test]
async fn native_schedule_queues_cancels_edits_fires_and_never_recreates_deleted_target() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("conversations")).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    std::fs::write(dir.path().join("settings.json"),json!({"settings":{"apiRetryEnabled":false,"chat":{"defaultAgentRuntime":{"kind":"chat"}}}}).to_string()).unwrap();
    std::fs::write(dir.path().join("conversations/providers.json"),json!({
        "providers":[{"id":"fixture","name":"Fixture","baseUrl":base,"apiFormat":"openai_chat","enabled":true,"apiKeys":[],"availableModels":["fixture-model"],"enabledModels":["fixture-model"],"request":{"useSystemProxy":false}}],
        "defaultModels":{"chat":{"providerId":"fixture","model":"fixture-model"}}}).to_string()).unwrap();
    kivio_model_runtime::native_preferences::save(
        dir.path(),
        kivio_model_runtime::native_preferences::Change::Retry(false),
    )
    .unwrap();
    let (release, held) = std::sync::mpsc::channel();
    let (requests, received) = std::sync::mpsc::channel();
    let server = std::thread::spawn(move || {
        for ix in 0..3 {
            let (mut stream, body) = loop {
                let (mut stream, _) = listener.accept().unwrap();
                if let Some(body) = request(&mut stream) {
                    break (stream, body);
                }
                stream
                    .write_all(
                        b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    )
                    .unwrap();
            };
            requests.send(body).unwrap();
            if ix == 0 {
                held.recv_timeout(Duration::from_secs(15)).unwrap();
            }
            let body = "data: {\"choices\":[{\"delta\":{\"content\":\"Native task completed\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n";
            write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).unwrap();
        }
    });
    let handle = EngineHandle::bootstrap(EngineBootConfig {
        data_dir: dir.path().into(),
        ipc_port: 0,
        edge_url: String::new(),
        edge_token: None,
        org_id: None,
        workos_client_id: None,
        default_harness: HarnessId::Codex,
    })
    .await
    .unwrap();
    let service = handle.kivio_service().unwrap();
    let owner = service.scheduled_tasks().unwrap();
    let task = service.save_scheduled(draft()).unwrap();
    assert!(
        dir.path().join("scheduled_tasks/tasks.json").is_file(),
        "retain the original task directory contract"
    );
    assert!(!dir.path().join("scheduled-tasks").exists());
    let id = task.conversation_id.clone();
    let store = ConversationStore::new(dir.path().join("conversations"));
    assert_eq!(store.load(&id).unwrap().title, "Native schedule");
    handle
        .client()
        .call("KivioSend", json!({"chatId":id,"text":"hold user reply"}))
        .await
        .unwrap();
    wait(|| received.try_recv().is_ok()).await;
    let queued = service.run_scheduled(&task.id).unwrap();
    assert_eq!(queued.status, RunStatus::Queued);
    let mut edited = draft();
    edited.id = Some(task.id.clone());
    edited.prompt = "replacement scheduled prompt".into();
    edited.target = TaskTarget::Conversation {
        conversation_id: id.clone(),
    };
    service.save_scheduled(edited).unwrap();
    wait(|| {
        owner
            .runs(&task.id)
            .unwrap()
            .iter()
            .any(|r| r.id == queued.id && r.status == RunStatus::Skipped)
    })
    .await;
    assert_eq!(
        store.load(&id).unwrap().messages.len(),
        1,
        "cancelled queued prompt must not be saved as a user message"
    );
    release.send(()).unwrap();
    wait(|| store.load(&id).is_ok_and(|c| c.messages.len() == 2)).await;
    let run = service.run_scheduled(&task.id).unwrap();
    wait(|| {
        owner
            .runs(&task.id)
            .unwrap()
            .iter()
            .any(|r| r.id == run.id && r.status == RunStatus::Succeeded)
    })
    .await;
    let body = received.try_recv().unwrap();
    assert!(body.contains("replacement scheduled prompt"));
    assert!(!body.contains("old scheduled prompt"));
    assert_eq!(store.load(&id).unwrap().messages.len(), 4);
    let c = store.load(&id).unwrap();
    store.delete(&id, c.revision).unwrap();
    let run = service.run_scheduled(&task.id).unwrap();
    wait(|| {
        owner
            .runs(&task.id)
            .unwrap()
            .iter()
            .any(|r| r.id == run.id && r.status == RunStatus::Failed)
    })
    .await;
    assert!(!owner.get(&task.id).unwrap().enabled);
    assert!(store.load(&id).is_err());
    let mut automatic = draft();
    automatic.schedule = ScheduleRule::Once {
        at: scheduled_tasks::now_secs() + 2,
    };
    let automatic = service.save_scheduled(automatic).unwrap();
    wait(|| {
        owner
            .runs(&automatic.id)
            .unwrap()
            .iter()
            .any(|r| r.trigger == RunTrigger::Schedule && r.status == RunStatus::Succeeded)
    })
    .await;
    assert_eq!(
        owner.get(&automatic.id).unwrap().status,
        TaskStatus::Completed
    );
    assert!(
        received
            .try_recv()
            .unwrap()
            .contains("old scheduled prompt")
    );
    service.shutdown().await;
    server.join().unwrap();
}
