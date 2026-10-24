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
    use kivio_model_runtime::native_preferences::{self, Change, TextPreference};
    std::fs::write(
        dir.path().join("settings.json"),
        br#"{"other":{"keep":true},"settings":{"unknownFutureField":42}}"#,
    )
    .unwrap();
    let text_change = |field: TextPreference, after: &str| Change::Text {
        field,
        before: field.value(&native_preferences::load(dir.path()).unwrap()),
        after: after.into(),
    };
    native_preferences::save(
        dir.path(),
        text_change(TextPreference::AgentPrompt, "KIVIO_SETTINGS_WIRED"),
    )
    .unwrap();
    native_preferences::save(dir.path(), Change::Retry(false)).unwrap();
    native_preferences::save(dir.path(), Change::Thinking(false)).unwrap();
    let persisted: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.path().join("settings.json")).unwrap()).unwrap();
    assert_eq!(persisted["other"]["keep"], true);
    assert_eq!(persisted["settings"]["unknownFutureField"], 42);
    native_preferences::save(
        dir.path(),
        text_change(TextPreference::LensQuestionPrompt, "Lens-only prompt"),
    )
    .unwrap();
    native_preferences::save(
        dir.path(),
        text_change(TextPreference::ChatPrompt, "Chat-only instructions"),
    )
    .unwrap();
    let reopened = native_preferences::load(dir.path()).unwrap();
    assert_eq!(reopened.chat.system_prompt, "KIVIO_SETTINGS_WIRED");
    assert_eq!(
        reopened.chat.chat_mode.system_prompt,
        "Chat-only instructions"
    );
    assert_eq!(reopened.lens.question_prompt, "Lens-only prompt");
    let before = std::fs::read(dir.path().join("settings.json")).unwrap();
    assert!(
        native_preferences::save(
            dir.path(),
            text_change(TextPreference::LensOrder, "invalid")
        )
        .is_err()
    );

    assert!(native_preferences::save(dir.path(), Change::RetryAttempts(0)).is_err());
    assert_eq!(
        std::fs::read(dir.path().join("settings.json")).unwrap(),
        before
    );
    assert!(
        !native_preferences::load(dir.path())
            .unwrap()
            .chat
            .thinking_enabled
    );
    let memory_dir = kivio_model_runtime::memory_files::memory_dir_at(dir.path()).unwrap();
    kivio_model_runtime::memory_files::save_layer_at(
        &memory_dir,
        kivio_model_runtime::memory_files::MemoryLayer::L1,
        "native memory belongs in requests",
    )
    .unwrap();
    native_preferences::save(
        dir.path(),
        Change::Fields(vec![native_preferences::FieldEdit {
            path: "/chatMemory/enabled".into(),
            before: json!(false),
            after: json!(true),
        }]),
    )
    .unwrap();
    #[cfg(unix)]
    {
        native_preferences::save(
            dir.path(),
            Change::Fields(vec![native_preferences::FieldEdit {
                path: "/chatTools/nativeTools/workingDirectory".into(),
                before: json!(
                    native_preferences::load(dir.path())
                        .unwrap()
                        .chat_tools
                        .native_tools
                        .working_directory
                ),
                after: json!(dir.path()),
            }]),
        )
        .unwrap();
        for event in [
            "agent_start",
            "turn_start",
            "message_start",
            "message_end",
            "turn_end",
            "agent_end",
        ] {
            native_preferences::save(
                dir.path(),
                Change::Hook {
                    before: None,
                    after: Some(kivio_model_runtime::tool_settings::HookDef {
                        id: event.into(),
                        name: event.into(),
                        event: event.into(),
                        enabled: true,
                        kind: "command".into(),
                        script: "cat >> hook-events.jsonl".into(),
                        ..Default::default()
                    }),
                },
            )
            .unwrap();
        }
    }
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
    let fixture_address = listener.local_addr().unwrap();
    let base = format!("http://{fixture_address}/v1");
    std::fs::create_dir_all(dir.path().join("conversations")).unwrap();
    let project_root = dir.path().join("project-workspace");
    std::fs::create_dir(&project_root).unwrap();
    std::fs::write(dir.path().join("conversations/projects.json"), json!({"projects":[{"id":"proj_hook","name":"Hook project","root_path":project_root,"created_at":0,"updated_at":0}]}).to_string()).unwrap();
    std::fs::write(dir.path().join("conversations/providers.json"),serde_json::to_vec(&json!({
        "providers":[{"id":"fixture","name":"Local test","baseUrl":base,"apiFormat":"openai_chat","apiKeys":[],"enabled":true,"availableModels":["fixture-model"],"enabledModels":["fixture-model"],"modelOverrides":{"fixture-model":{"temperature":0.3,"maxOutput":512,"extraBody":{"native_settings_override":true}}},"request":{"useSystemProxy":false}}],
        "defaultModels":{"chat":{"providerId":"fixture","model":"fixture-model"}}
    })).unwrap()).unwrap();
    assert!(handle.client().call("KivioSend", json!({"chatId":"conv_bad_attachment","text":"bad image","attachments":[{"name":"bad.png","data":"bm90IGFuIGltYWdl"}]})).await.is_err());
    assert!(
        store.load_all().unwrap().is_empty(),
        "media rejection must precede conversation creation"
    );
    assert!(
        !dir.path()
            .join("conversations/conv_bad_attachment_attachments")
            .exists(),
        "rejected copies must be cleaned up"
    );
    let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
    let server_progress = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let progress = server_progress.clone();
    let server = std::thread::spawn(move || {
        let mut requests = Vec::new();
        while requests.len() < 2 {
            let turn = requests.len();
            let (mut socket, _) = listener.accept().unwrap();
            progress.lock().unwrap().push(format!("turn {turn}: accepted"));
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
            let request_line = String::from_utf8_lossy(&request)
                .lines()
                .next()
                .unwrap_or_default()
                .to_owned();
            progress
                .lock()
                .unwrap()
                .push(format!("turn {turn}: {request_line}"));
            // Local port discovery may probe this listener with HEAD /. Only
            // actual model calls consume a scripted reply; otherwise a probe
            // can steal the partial response and block the real second turn.
            if request_line != "POST /v1/chat/completions HTTP/1.1" {
                socket.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
                continue;
            }
            let body = "data: {\"id\":\"fixture\",\"object\":\"chat.completion.chunk\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"接线成功\"},\"finish_reason\":null}]}\n\ndata: {\"id\":\"fixture\",\"object\":\"chat.completion.chunk\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n";
            if turn == 1 {
                // Send a normal sentence: the shared Agent sink briefly holds
                // short planning prefixes to detect tool markup before painting.
                let partial = "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"尚未完成，正在继续处理请求。\"},\"finish_reason\":null}]}\n\n";
                write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: 99999\r\n\r\n{partial}").unwrap();
                socket.flush().unwrap();
                progress
                    .lock()
                    .unwrap()
                    .push(format!("turn {turn}: partial flushed"));
                let _ = release_rx.recv_timeout(Duration::from_secs(15));
                requests.push(request);
                continue;
            }
            write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).unwrap();
            requests.push(request);
        }
        requests
    });
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    let reply = handle
        .client()
        .call("KivioSend", json!({"chatId":"conv_port","projectId":"proj_hook","text":"你好","attachments":[{"name":"note.txt","data":STANDARD.encode("attachment body 123") },{"name":"pixel.png","data":"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg=="}]}))
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
    .unwrap_or_else(|error| {
        panic!(
            "reply timeout: {error}; server={:?}; conversation={:?}; hooks={:?}",
            server_progress.lock().unwrap(),
            store.load("conv_port"),
            std::fs::read_to_string(project_root.join("hook-events.jsonl"))
        )
    });
    let saved = store.load("conv_port").unwrap();
    assert_eq!(saved.messages[0].content, "你好");
    assert_eq!(saved.messages[0].attachments.len(), 2);
    let attachment_path = dir
        .path()
        .join("conversations/conv_port_attachments")
        .join(&saved.messages[0].attachments[0].path);
    assert_eq!(
        std::fs::read_to_string(&attachment_path).unwrap(),
        "attachment body 123"
    );
    let read = handle
        .client()
        .call(
            "ReadAttachmentChunk",
            json!({"path":attachment_path,"offset":0}),
        )
        .await
        .unwrap();
    assert_eq!(
        STANDARD.decode(read["data"].as_str().unwrap()).unwrap(),
        b"attachment body 123"
    );
    assert!(
        handle
            .client()
            .call(
                "ReadAttachmentChunk",
                json!({"path":dir.path().join("conversations/providers.json")})
            )
            .await
            .is_err()
    );
    assert_eq!(saved.messages[1].content, "接线成功");
    let stats = kivio_model_runtime::usage_ledger::stats_in_directories(
        &[
            dir.path().join("conversations/conv_port/usage"),
            dir.path().join("usage"),
        ],
        kivio_model_runtime::usage_ledger::UsageStatsQuery {
            range: "all".into(),
            ..Default::default()
        },
    );
    assert_eq!(stats.summary.total_requests, 1);
    assert_eq!(stats.logs[0].model, "fixture-model");

    assert_eq!(
        saved.messages[1].stream_outcome.as_deref(),
        Some("completed")
    );
    let mut history = store.load("conv_port").unwrap();
    history.messages[1].tool_calls.push(serde_json::from_value(json!({"id":"tool_old","name":"read","arguments":"{\"path\":\"note.txt\"}","status":"success","round":1,"result_preview":"saved tool output"})).unwrap());
    let revision = history.revision;
    store.save(history, revision).unwrap();
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
    let tool = &frame["reset"][1]["parts"][1];
    assert_eq!(tool["kind"], "tool");
    assert_eq!(tool["resolved"], true);
    assert_eq!(tool["call"]["input"]["path"], "note.txt");
    assert_eq!(tool["output"], "saved tool output");
    #[cfg(unix)]
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let lines =
                std::fs::read_to_string(project_root.join("hook-events.jsonl")).unwrap_or_default();
            let events: Vec<serde_json::Value> = lines
                .lines()
                .filter_map(|line| serde_json::from_str(line).ok())
                .collect();
            assert!(!dir.path().join("hook-events.jsonl").exists(), "project Hook ran in the global working directory");
            if events.len() >= 6 {
                assert_eq!(
                    events
                        .iter()
                        .map(|event| event["event"].as_str().unwrap())
                        .collect::<Vec<_>>(),
                    [
                        "agent_start",
                        "turn_start",
                        "message_start",
                        "message_end",
                        "turn_end",
                        "agent_end"
                    ]
                );
                assert!(
                    events
                        .iter()
                        .all(|event| event["conversationId"] == "conv_port")
                );
                assert!(events.iter().all(|event| event["cwd"] == project_root.to_string_lossy().as_ref()));
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("native lifecycle hooks did not complete");
    native_preferences::save(
        dir.path(),
        Change::Fields(vec![native_preferences::FieldEdit {
            path: "/chatMemory/enabled".into(),
            before: json!(true),
            after: json!(false),
        }]),
    )
    .unwrap();
    // Reproduce the unrelated local HEAD probe that previously stole turn 2.
    tokio::task::spawn_blocking(move || {
        let mut probe = std::net::TcpStream::connect(fixture_address).unwrap();
        probe
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        probe.write_all(b"HEAD / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n").unwrap();
        let mut response = String::new();
        probe.read_to_string(&mut response).unwrap();
        assert!(response.starts_with("HTTP/1.1 404 Not Found\r\n"), "{response}");
    })
    .await
    .unwrap();
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
    let mut observed_frames = Vec::new();
    let partial = tokio::time::timeout(Duration::from_secs(10), async {
        while let Some(frame) = live.recv().await {
            let found = frame.to_string().contains("尚未完成，正在继续处理请求。");
            observed_frames.push(frame);
            if found {
                return true;
            }
        }
        false
    })
    .await;
    let fresh_frame = if !matches!(partial, Ok(true)) {
        let mut fresh = reopened
            .client()
            .subscribe("WatchDocMessages", json!({"chatId":"conv_cancel"}))
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(1), fresh.recv())
            .await
            .ok()
            .flatten()
    } else {
        None
    };
    assert!(
        matches!(partial, Ok(true)),
        "partial stream missing: {partial:?}; server={:?}; frames={observed_frames:?}; fresh={fresh_frame:?}; conversation={:?}",
        server_progress.lock().unwrap(),
        store.load("conv_cancel"),
    );
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
    assert!(reopened.client().call("Mutate", json!({"op":"moveChat","chatId":"conv_cancel","groupKind":"project","groupId":"proj_hook"})).await.is_err(), "running reply must retain its execution context");
    assert_eq!(store.load("conv_cancel").unwrap().project_id, None);
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
        .call(
            "Mutate",
            json!({"op":"setChatPinned", "chatId":"conv_cancel", "pinned":true}),
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
    assert!(
        cancelled.pinned,
        "reply finalization must preserve a concurrent library favorite change"
    );
    assert_eq!(cancelled.messages[1].content, "尚未完成，正在继续处理请求。");
    assert_eq!(
        cancelled.messages[1].stream_outcome.as_deref(),
        Some("cancelled")
    );
    // The native library must show active history by default and keep the
    // archive shelf separate, using the same query as the installed client.
    use kivio_model_runtime::{
        conversation_data::ConversationListItem,
        conversation_library::{ConversationLibraryQuery, query_items},
    };
    let mut archived = store.load("conv_port").unwrap();
    archived.archived = true;
    let revision = archived.revision;
    store.save(archived, revision).unwrap();
    let items = store
        .load_all()
        .unwrap()
        .iter()
        .map(ConversationListItem::from)
        .collect::<Vec<_>>();
    let active = query_items(
        items.clone(),
        ConversationLibraryQuery {
            limit: 40,
            shelf: "all".into(),
            ..Default::default()
        },
        |_, _| false,
        |_, _| None,
    )
    .unwrap();
    assert_eq!(active.total, 1);
    assert_eq!(active.items[0].item.id, "conv_cancel");
    let archive = query_items(
        items,
        ConversationLibraryQuery {
            limit: 40,
            shelf: "archived".into(),
            ..Default::default()
        },
        |_, _| false,
        |_, _| None,
    )
    .unwrap();
    assert_eq!(archive.total, 1);
    assert_eq!(archive.items[0].item.id, "conv_port");
    release_tx.send(()).unwrap();
    let requests = server.join().unwrap();
    assert_eq!(requests.len(), 2);
    let body_start = requests[0]
        .windows(4)
        .position(|bytes| bytes == b"\r\n\r\n")
        .unwrap()
        + 4;
    let body: serde_json::Value = serde_json::from_slice(&requests[0][body_start..]).unwrap();
    assert_eq!(body["temperature"], json!(0.3));
    assert_eq!(body["native_settings_override"], true);
    assert_eq!(
        body.get("max_tokens")
            .or_else(|| body.get("max_completion_tokens")),
        Some(&json!(512))
    );

    assert!(String::from_utf8_lossy(&requests[0]).contains("native memory belongs in requests"));
    assert!(!String::from_utf8_lossy(&requests[1]).contains("native memory belongs in requests"));
    assert!(String::from_utf8_lossy(&requests[0]).contains("fixture-model"));
    assert!(String::from_utf8_lossy(&requests[0]).contains("KIVIO_SETTINGS_WIRED"));
    assert!(
        String::from_utf8_lossy(&requests[0]).contains("data:image/png;base64,"),
        "image bytes must reach the model"
    );
    assert!(
        String::from_utf8_lossy(&requests[0]).contains("conv_port_attachments"),
        "documents must carry the existing safe-copy tool reference"
    );
    reopened.shutdown().await;
}

#[tokio::test]
async fn imported_terminal_replays_resizes_and_closes_real_pty() {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    let dir = tempfile::tempdir().unwrap();
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
    let client = handle.client();
    assert!(
        client
            .call("OpenTerminal", json!({"cwd":dir.path().join("missing")}))
            .await
            .is_err()
    );
    let opened = client
        .call(
            "OpenTerminal",
            json!({"cwd":dir.path(),"cols":80,"rows":24}),
        )
        .await
        .unwrap();
    let id = opened["id"].as_str().unwrap();
    client
        .call(
            "ResizeTerminal",
            json!({"terminalId":id,"cols":93,"rows":31}),
        )
        .await
        .unwrap();
    // Write before subscribing: startup/early output must survive the handoff.
    client.call("WriteTerminal", json!({"terminalId":id,"data":STANDARD.encode(b"printf '\\113\\111\\126\\111\\117_OK\\n'; stty size\n")})).await.unwrap();
    let mut stream = client
        .subscribe("SubscribeTerminal", json!({"terminalId":id,"afterSeq":0}))
        .await
        .unwrap();
    let mut output = String::new();
    let mut seq = 0;
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let frame = stream.recv().await.unwrap();
            assert!(frame["seq"].as_u64().unwrap() > seq);
            seq = frame["seq"].as_u64().unwrap();
            if let Some(data) = frame["data"].as_str() {
                output.push_str(&String::from_utf8_lossy(&STANDARD.decode(data).unwrap()));
            }
            if output.contains("KIVIO_OK") && output.contains("31 93") {
                break;
            }
        }
    })
    .await
    .expect("real PTY must execute and report its resized geometry");
    drop(stream);
    let mut replay = client
        .subscribe("SubscribeTerminal", json!({"terminalId":id,"afterSeq":seq}))
        .await
        .unwrap();
    client
        .call("CloseTerminal", json!({"terminalId":id}))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        while let Some(frame) = replay.recv().await {
            if frame["type"] == "exit" {
                return;
            }
        }
        panic!("close must terminate the subscription");
    })
    .await
    .unwrap();
    assert!(
        client
            .call(
                "WriteTerminal",
                json!({"terminalId":id,"data":STANDARD.encode(b"pwd\n")})
            )
            .await
            .is_err()
    );
    handle.shutdown().await;
}

#[test]
fn native_memory_preserves_layers_and_rejects_over_quota_edits() {
    use kivio_model_runtime::memory_files::{self, MemoryLayer};
    let root = tempfile::tempdir().unwrap();
    let directory = memory_files::memory_dir_at(root.path()).unwrap();
    memory_files::save_layer_at(&directory, MemoryLayer::L1, "当前项目使用 Rust。\n").unwrap();
    memory_files::save_layer_at(&directory, MemoryLayer::L2, "长期资料。\n").unwrap();
    let over_limit = "中".repeat(1667);
    assert!(memory_files::save_layer_at(&directory, MemoryLayer::L1, &over_limit).is_err());
    assert_eq!(
        memory_files::read_layer_at(&directory, MemoryLayer::L1)
            .unwrap()
            .content,
        "当前项目使用 Rust。\n"
    );
    assert_eq!(
        memory_files::read_layer_at(&directory, MemoryLayer::L2)
            .unwrap()
            .content,
        "长期资料。\n"
    );
}

#[test]
fn native_hooks_save_rejects_stale_edits_and_preserves_other_settings() {
    use kivio_model_runtime::{
        native_preferences::{self, Change},
        tool_settings::HookDef,
    };
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("settings.json"),
        br#"{"settings":{"chatTools":{"futureFlag":true},"unknown":42}}"#,
    )
    .unwrap();
    let hook = HookDef {
        id: "fixture-hook".into(),
        name: "Observe".into(),
        event: "agent_start".into(),
        script: "printf done".into(),
        ..Default::default()
    };
    native_preferences::save(
        dir.path(),
        Change::Hook {
            before: None,
            after: Some(hook.clone()),
        },
    )
    .unwrap();
    let mut updated = hook.clone();
    updated.description = "Changed elsewhere".into();
    native_preferences::save(
        dir.path(),
        Change::Hook {
            before: Some(hook.clone()),
            after: Some(updated.clone()),
        },
    )
    .unwrap();
    let path = dir.path().join("settings.json");
    let snapshot = std::fs::read(&path).unwrap();
    assert!(
        native_preferences::save(
            dir.path(),
            Change::Hook {
                before: Some(hook.clone()),
                after: None
            }
        )
        .is_err()
    );
    assert_eq!(std::fs::read(&path).unwrap(), snapshot);
    let mut invalid = updated.clone();
    invalid.timeout_ms = 0;
    assert!(
        native_preferences::save(
            dir.path(),
            Change::Hook {
                before: Some(updated.clone()),
                after: Some(invalid)
            }
        )
        .is_err()
    );
    assert_eq!(std::fs::read(&path).unwrap(), snapshot);
    assert_eq!(
        native_preferences::load(dir.path())
            .unwrap()
            .chat_tools
            .hooks[0]
            .description,
        "Changed elsewhere"
    );
    native_preferences::save(
        dir.path(),
        Change::Hook {
            before: Some(updated),
            after: None,
        },
    )
    .unwrap();
    let saved: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(saved["settings"]["chatTools"]["hooks"], json!([]));
    assert_eq!(saved["settings"]["chatTools"]["futureFlag"], true);
    assert_eq!(saved["settings"]["unknown"], 42);
}

#[test]
fn native_search_saves_only_changed_fields_and_rejects_stale_credentials() {
    use kivio_model_runtime::native_preferences::{self, Change};
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("settings.json"),
        br#"{"settings":{"lens":{"webSearch":{"futureOption":true}}}}"#,
    )
    .unwrap();
    let baseline = native_preferences::load(dir.path())
        .unwrap()
        .lens
        .web_search;
    let mut credentials = baseline.clone();
    credentials.tavily_api_key = "fixture-secret".into();
    native_preferences::save(
        dir.path(),
        Change::WebSearch {
            before: baseline.clone(),
            after: credentials.clone(),
        },
    )
    .unwrap();
    let mut defaults = baseline.clone();
    defaults.max_results = 8;
    native_preferences::save(
        dir.path(),
        Change::WebSearch {
            before: baseline.clone(),
            after: defaults,
        },
    )
    .unwrap();
    let reopened = native_preferences::load(dir.path())
        .unwrap()
        .lens
        .web_search;
    assert_eq!(reopened.max_results, 8);
    assert_eq!(reopened.tavily_api_key, "fixture-secret");
    let path = dir.path().join("settings.json");
    let snapshot = std::fs::read(&path).unwrap();
    let mut stale = baseline.clone();
    stale.tavily_api_key = "stale-secret".into();
    assert!(
        native_preferences::save(
            dir.path(),
            Change::WebSearch {
                before: baseline,
                after: stale
            }
        )
        .is_err()
    );
    assert_eq!(std::fs::read(&path).unwrap(), snapshot);
    let mut invalid = reopened.clone();
    invalid.max_results = 11;
    assert!(
        native_preferences::save(
            dir.path(),
            Change::WebSearch {
                before: reopened,
                after: invalid
            }
        )
        .is_err()
    );
    assert_eq!(std::fs::read(&path).unwrap(), snapshot);
    let saved: serde_json::Value = serde_json::from_slice(&snapshot).unwrap();
    assert_eq!(saved["settings"]["lens"]["webSearch"]["futureOption"], true);
    let before = native_preferences::load(dir.path())
        .unwrap()
        .lens
        .web_search;
    let mut after = before.clone();
    after.fetch_provider = Some(kivio_model_runtime::settings_config::WebSearchProvider::Exa);
    native_preferences::save(
        dir.path(),
        Change::WebSearch {
            before,
            after: after.clone(),
        },
    )
    .unwrap();
    let mut follow_search = after.clone();
    follow_search.fetch_provider = None;
    native_preferences::save(
        dir.path(),
        Change::WebSearch {
            before: after,
            after: follow_search,
        },
    )
    .unwrap();
    assert!(
        native_preferences::load(dir.path())
            .unwrap()
            .lens
            .web_search
            .fetch_provider
            .is_none()
    );
}

#[test]
fn settings_field_edits_preserve_other_pages_and_reject_stale_or_invalid_values() {
    use kivio_model_runtime::native_preferences::{self, Change, FieldEdit};
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("settings.json"),
        r#"{"metadata":{"future":true},"settings":{"lens":{"futureField":42}}}"#,
    )
    .unwrap();
    let edit = |path: &str, before, after| FieldEdit {
        path: path.into(),
        before,
        after,
    };
    native_preferences::save(
        dir.path(),
        Change::Fields(vec![edit(
            "/lens/messageOrder",
            json!("asc"),
            json!("desc"),
        )]),
    )
    .unwrap();
    native_preferences::save(
        dir.path(),
        Change::Fields(vec![edit(
            "/screenshotTranslation/cardWidth",
            json!(480),
            json!(600),
        )]),
    )
    .unwrap();
    let reopened = native_preferences::load(dir.path()).unwrap();
    assert_eq!(reopened.lens.message_order, "desc");
    assert_eq!(reopened.screenshot_translation.card_width, 600);
    let bytes = std::fs::read(dir.path().join("settings.json")).unwrap();
    let raw: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(raw["metadata"]["future"], true);
    assert_eq!(raw["settings"]["lens"]["futureField"], 42);
    for edits in [
        vec![edit("/lens/messageOrder", json!("asc"), json!("asc"))],
        vec![edit(
            "/screenshotTranslation/cardWidth",
            json!(600),
            json!(999),
        )],
        vec![edit("/lens/enabled", json!(true), json!("true"))],
        vec![
            edit("/lens/messageOrder", json!("desc"), json!("asc")),
            edit("/screenshotTranslation/cardWidth", json!(600), json!(20)),
        ],
    ] {
        assert!(native_preferences::save(dir.path(), Change::Fields(edits)).is_err());
        assert_eq!(
            std::fs::read(dir.path().join("settings.json")).unwrap(),
            bytes
        );
    }
    // Text editors have their own confirmed-value CAS, including optional prompts.
    use kivio_model_runtime::native_preferences::TextPreference;
    let field = TextPreference::InputPrompt;
    let baseline = field.value(&native_preferences::load(dir.path()).unwrap());
    assert_eq!(baseline, serde_json::Value::Null);
    let other_prompt = TextPreference::LensQuestionPrompt;
    let other_before = other_prompt.value(&native_preferences::load(dir.path()).unwrap());
    native_preferences::save(dir.path(), Change::Text { field, before: baseline.clone(), after: "new prompt from another editor".into() }).unwrap();
    // A changed sibling does not prevent saving a disjoint editor.
    native_preferences::save(dir.path(), Change::Text { field: other_prompt, before: other_before, after: "independent Lens prompt".into() }).unwrap();
    let saved = std::fs::read(dir.path().join("settings.json")).unwrap();
    assert!(native_preferences::save(dir.path(), Change::Text { field, before: baseline, after: "stale editor overwrite".into() }).is_err());
    assert_eq!(std::fs::read(dir.path().join("settings.json")).unwrap(), saved);
    let current = field.value(&native_preferences::load(dir.path()).unwrap());
    native_preferences::save(dir.path(), Change::Text { field, before: current, after: String::new() }).unwrap();
    assert_eq!(native_preferences::load(dir.path()).unwrap().translator_prompt.as_deref(), Some(""));

    // Legacy reading-language inheritance must not follow a later input-language edit.
    let mut legacy: serde_json::Value = serde_json::from_slice(&std::fs::read(dir.path().join("settings.json")).unwrap()).unwrap();
    legacy["settings"]["readingTargetLang"] = json!("");
    legacy["settings"]["targetLang"] = json!("en");
    std::fs::write(dir.path().join("settings.json"), legacy.to_string()).unwrap();
    let opened = native_preferences::load(dir.path()).unwrap();
    assert_eq!(opened.reading_target_lang, "en");
    native_preferences::save(dir.path(), Change::Text { field: TextPreference::InputLanguage, before: TextPreference::InputLanguage.value(&opened), after: "ja".into() }).unwrap();
    let changed = native_preferences::load(dir.path()).unwrap();
    assert_eq!(changed.target_lang, "ja");
    assert_eq!(changed.reading_target_lang, "en");
    native_preferences::save(dir.path(), Change::Text { field: TextPreference::ReadingLanguage, before: TextPreference::ReadingLanguage.value(&opened), after: "de".into() }).unwrap();
    let saved = std::fs::read(dir.path().join("settings.json")).unwrap();
    assert!(native_preferences::save(dir.path(), Change::Text { field: TextPreference::ReadingLanguage, before: TextPreference::ReadingLanguage.value(&opened), after: "fr".into() }).is_err());
    assert!(native_preferences::save(dir.path(), Change::Text { field: TextPreference::ReadingLanguage, before: json!("de"), after: "auto".into() }).is_err());
    assert_eq!(std::fs::read(dir.path().join("settings.json")).unwrap(), saved);

}

#[test]
fn custom_theme_roundtrip_validates_original_document_and_detects_edit_conflicts() {
    use kivio_model_runtime::{
        native_preferences::{self, Change},
        native_themes,
    };
    let directory = tempfile::tempdir().unwrap();
    let builtins: serde_json::Value =
        serde_json::from_str(include_str!("../../src/theme/builtins.json")).unwrap();
    let mut document = json!({"version":1,"theme":{"name":"原版主题","light":builtins[0]["light"],"dark":builtins[0]["dark"]}});
    assert_eq!(
        native_preferences::appearance_selection(directory.path()).unwrap(),
        (None, None)
    );
    let theme = native_themes::parse(&document.to_string()).unwrap();
    native_preferences::save(
        directory.path(),
        Change::Theme {
            before: None,
            after: Some(theme.clone()),
        },
    )
    .unwrap();
    let settings = native_preferences::load(directory.path()).unwrap();
    native_preferences::save(
        directory.path(),
        Change::Fields(vec![
            native_preferences::FieldEdit {
                path: "/theme".into(),
                before: json!(settings.theme),
                after: json!("dark"),
            },
            native_preferences::FieldEdit {
                path: "/themeColor".into(),
                before: json!(settings.theme_color),
                after: json!(theme.id),
            },
        ]),
    )
    .unwrap();
    assert_eq!(
        native_preferences::appearance_selection(directory.path()).unwrap(),
        (Some("dark".into()), Some(theme.id.clone()))
    );
    let file = directory.path().join("theme.json");
    native_themes::export(&theme, &file).unwrap();
    let imported = native_themes::parse(&std::fs::read_to_string(file).unwrap()).unwrap();
    assert_ne!(imported.id, theme.id);
    assert_eq!(imported.light, theme.light);
    assert_eq!(imported.dark, theme.dark);
    let mut updated = theme.clone();
    updated.name = "已编辑".into();
    native_preferences::save(
        directory.path(),
        Change::Theme {
            before: Some(theme.clone()),
            after: Some(updated.clone()),
        },
    )
    .unwrap();
    assert!(
        native_preferences::save(
            directory.path(),
            Change::Theme {
                before: Some(theme),
                after: None
            }
        )
        .is_err()
    );
    assert_eq!(
        native_preferences::load(directory.path())
            .unwrap()
            .custom_themes,
        vec![updated]
    );
    document["theme"]["light"]["accent"] = json!("not-a-color");
    assert!(native_themes::parse(&document.to_string()).is_err());
    document["theme"]["light"]["accent"] = builtins[0]["light"]["accent"].clone();
    document["theme"]["unexpected"] = json!(true);
    assert!(native_themes::parse(&document.to_string()).is_err());
    let before = native_preferences::load(directory.path())
        .unwrap()
        .chat_tools
        .skill_scan_paths;
    let after = vec![directory.path().to_string_lossy().into_owned()];
    native_preferences::save(
        directory.path(),
        Change::Fields(vec![native_preferences::FieldEdit {
            path: "/chatTools/skillScanPaths".into(),
            before: json!(before),
            after: json!(after),
        }]),
    )
    .unwrap();
    assert_eq!(
        native_preferences::load(directory.path())
            .unwrap()
            .chat_tools
            .skill_scan_paths,
        after
    );
}
