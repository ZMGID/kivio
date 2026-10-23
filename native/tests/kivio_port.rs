//! Boundary checks for the real imported-UI → Kivio store/provider route.
use kivio_model_runtime::conversation_files::ConversationStore;
use serde_json::json;
use std::{
    io::{Read, Write},
    net::TcpListener,
    time::Duration,
};
use zeron_ui::{EngineBootConfig, HarnessId, state::EngineHandle};

#[tokio::test]
async fn imported_ui_send_persists_and_reopens_kivio_history() {
    let dir = tempfile::tempdir().unwrap();
    let boot = EngineBootConfig {
        data_dir: dir.path().into(),
        ipc_port: 0,
        edge_url: String::new(),
        edge_token: None,
        org_id: None,
        workos_client_id: None,
        default_harness: HarnessId::Codex,
    };
    let handle = EngineHandle::bootstrap(boot.clone()).await.unwrap();
    assert!(
        handle
            .client()
            .call("KivioSend", json!({"chatId":"conv_port","text":"你好"}))
            .await
            .is_err()
    );
    let store = ConversationStore::new(dir.path().join("conversations"));
    assert!(
        store.load_all().unwrap().is_empty(),
        "missing provider must not consume the draft into storage"
    );
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    std::fs::create_dir_all(dir.path().join("conversations")).unwrap();
    std::fs::write(dir.path().join("conversations/providers.json"),serde_json::to_vec(&json!({
        "providers":[{"id":"fixture","name":"Local test","baseUrl":base,"apiFormat":"openai_chat","apiKeys":[],"enabled":true,"availableModels":["fixture-model"],"enabledModels":["fixture-model"],"request":{"useSystemProxy":false}}],
        "defaultModels":{"chat":{"providerId":"fixture","model":"fixture-model"}}
    })).unwrap()).unwrap();
    let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
    let server = std::thread::spawn(move || {
        let mut requests = Vec::new();
        for turn in 0..2 {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(10)))
                .unwrap();
            let mut request = Vec::new();
            let mut buf = [0; 4096];
            loop {
                let n = socket.read(&mut buf).unwrap();
                assert!(n > 0);
                request.extend_from_slice(&buf[..n]);
                if let Some(end) = request.windows(4).position(|b| b == b"\r\n\r\n") {
                    let header = String::from_utf8_lossy(&request[..end]);
                    let size = header
                        .lines()
                        .find_map(|l| {
                            l.to_lowercase()
                                .strip_prefix("content-length:")
                                .map(|s| s.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    if request.len() >= end + 4 + size {
                        break;
                    }
                }
            }
            let body = "data: {\"id\":\"fixture\",\"object\":\"chat.completion.chunk\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"接线成功\"},\"finish_reason\":null}]}\n\ndata: {\"id\":\"fixture\",\"object\":\"chat.completion.chunk\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n";
            if turn == 1 {
                let partial = "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"尚未完成\"},\"finish_reason\":null}]}\n\n";
                write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: 99999\r\n\r\n{partial}").unwrap();
                socket.flush().unwrap();
                let _ = release_rx.recv_timeout(Duration::from_secs(15));
                requests.push(request);
                continue;
            }
            write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).unwrap();
            requests.push(request);
        }
        requests
    });
    let reply = handle
        .client()
        .call("KivioSend", json!({"chatId":"conv_port","text":"你好"}))
        .await
        .unwrap();
    assert_eq!(reply["chat"]["id"], "conv_port");
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if store.load("conv_port").unwrap().messages.len() == 2 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    let saved = store.load("conv_port").unwrap();
    assert_eq!(saved.messages[0].content, "你好");
    assert_eq!(saved.messages[1].content, "接线成功");
    assert_eq!(
        saved.messages[1].stream_outcome.as_deref(),
        Some("completed")
    );
    handle.shutdown().await;
    drop(handle);
    let reopened = EngineHandle::bootstrap(boot).await.unwrap();
    let mut messages = reopened
        .client()
        .subscribe("WatchDocMessages", json!({"chatId":"conv_port"}))
        .await
        .unwrap();
    let frame = messages.recv().await.unwrap();
    assert_eq!(frame["reset"][1]["parts"][0]["text"], "接线成功");
    reopened
        .client()
        .call("KivioSend", json!({"chatId":"conv_cancel","text":"继续"}))
        .await
        .unwrap();
    let mut live = reopened
        .client()
        .subscribe("WatchDocMessages", json!({"chatId":"conv_cancel"}))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        while let Some(frame) = live.recv().await {
            if frame.to_string().contains("尚未完成") {
                break;
            }
        }
    })
    .await
    .unwrap();
    assert!(
        reopened
            .client()
            .call(
                "KivioSend",
                json!({"chatId":"conv_cancel","text":"不能并发"})
            )
            .await
            .is_err()
    );
    reopened
        .client()
        .call(
            "Mutate",
            json!({"op":"renameChat","chatId":"conv_cancel","title":"运行中改名"}),
        )
        .await
        .unwrap();
    reopened
        .client()
        .call("KivioCancel", json!({"chatId":"conv_cancel"}))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if store.load("conv_cancel").unwrap().messages.len() == 2 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    let cancelled = store.load("conv_cancel").unwrap();
    assert_eq!(cancelled.title, "运行中改名");
    assert_eq!(cancelled.messages[1].content, "尚未完成");
    assert_eq!(
        cancelled.messages[1].stream_outcome.as_deref(),
        Some("cancelled")
    );
    release_tx.send(()).unwrap();
    let requests = server.join().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(String::from_utf8_lossy(&requests[0]).contains("fixture-model"));
    reopened.shutdown().await;
}
