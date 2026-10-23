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
    native_preferences::save(
        dir.path(),
        Change::Text(TextPreference::AgentPrompt, "KIVIO_SETTINGS_WIRED".into()),
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
        Change::Text(
            TextPreference::LensQuestionPrompt,
            "Lens-only prompt".into(),
        ),
    )
    .unwrap();
    native_preferences::save(
        dir.path(),
        Change::Text(TextPreference::ChatPrompt, "Chat-only instructions".into()),
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
            Change::Text(TextPreference::LensOrder, "invalid".into())
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
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    let reply = handle
        .client()
        .call("KivioSend", json!({"chatId":"conv_port","text":"你好","attachments":[{"name":"note.txt","data":STANDARD.encode("attachment body 123") },{"name":"pixel.png","data":"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg=="}]}))
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
