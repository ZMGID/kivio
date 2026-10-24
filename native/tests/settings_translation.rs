//! The native translation window uses saved settings and the original model wire path.
use kivio_model_runtime::native_translation;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
#[tokio::test]
async fn input_translation_reloads_selection_prompt_language_and_records_usage() {
    let root = tempfile::tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let mut bodies = vec![];
        for _ in 0..5 {
            let (mut socket, _) =
                tokio::time::timeout(std::time::Duration::from_secs(10), listener.accept())
                    .await
                    .unwrap()
                    .unwrap();
            let mut request = vec![];
            loop {
                let mut bytes = [0; 4096];
                let count = socket.read(&mut bytes).await.unwrap();
                assert!(count > 0);
                request.extend_from_slice(&bytes[..count]);
                if let Some(end) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
                    let header = String::from_utf8_lossy(&request[..end]);
                    let length: usize = header
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|size| size.trim().parse().unwrap())
                        })
                        .unwrap();
                    if request.len() >= end + 4 + length {
                        bodies.push(serde_json::from_slice::<Value>(&request[end + 4..]).unwrap());
                        break;
                    }
                }
            }
            if bodies.last().unwrap()["stream"] == true {
                let response = "data: {\"id\":\"selection-stream\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"bonjour\"},\"finish_reason\":null}]}\n\ndata: {\"id\":\"selection-stream\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n";
                let image_response = ["译文", "<<<ORI", "GINAL>>>", "原文"]
                    .into_iter().map(|delta| format!("data: {}\n\n", json!({"id":"image-stream","choices":[{"index":0,"delta":{"content":delta},"finish_reason":null}]})))
                    .collect::<String>() + "data: {\"id\":\"image-stream\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n";
                let response = if bodies.len() == 5 { image_response.as_str() } else { response };
                socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}", response.len()).as_bytes()).await.unwrap();
                continue;
            }
            let response = json!({"id":"translation-test","choices":[{"message":{"role":"assistant","content":" translated "},"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":2,"total_tokens":12}}).to_string();
            socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}", response.len()).as_bytes()).await.unwrap();
        }
        bodies
    });
    std::fs::create_dir(root.path().join("conversations")).unwrap();
    std::fs::write(root.path().join("conversations/providers.json"), json!({"providers":[{"id":"translator","name":"Fixture","enabled":true,"baseUrl":format!("http://{address}/v1"),"apiFormat":"openai_chat","enabledModels":["first","second"],"request":{"useSystemProxy":false}}]}).to_string()).unwrap();
    let mut settings = json!({"settings":{"translatorProviderId":"translator","translatorModel":"first","targetLang":"auto","translatorPrompt":"TARGET={lang}\nTEXT={text}","retryEnabled":false}});
    std::fs::write(root.path().join("settings.json"), settings.to_string()).unwrap();
    assert_eq!(
        native_translation::translate(root.path(), "   ")
            .await
            .unwrap(),
        ""
    );
    assert_eq!(
        native_translation::translate(root.path(), "你好")
            .await
            .unwrap(),
        "translated"
    );
    settings["settings"]["translatorModel"] = json!("second");
    settings["settings"]["targetLang"] = json!("ja");
    std::fs::write(root.path().join("settings.json"), settings.to_string()).unwrap();
    assert_eq!(
        native_translation::translate(root.path(), "hello")
            .await
            .unwrap(),
        "translated"
    );
    settings["settings"]["readingTargetLang"] = json!("fr");
    settings["settings"]["screenshotTranslation"] = json!({"providerId":"translator","model":"first","streamEnabled":false,"textPrompt":"SELECTED={text};TARGET={lang}"});
    std::fs::write(root.path().join("settings.json"), settings.to_string()).unwrap();
    assert_eq!(
        native_translation::translate_selection(root.path(), "selected words", &mut |_| Ok(()))
            .await
            .unwrap(),
        "translated"
    );
    settings["settings"]["screenshotTranslation"]["streamEnabled"] = json!(true);
    std::fs::write(root.path().join("settings.json"), settings.to_string()).unwrap();
    let mut deltas = String::new();
    let mut sink = |part| {
        if let kivio_model_runtime::StreamPart::TextDelta { delta } = part {
            deltas.push_str(&delta);
        }
        Ok(())
    };
    assert_eq!(
        native_translation::translate_selection(root.path(), "hello", &mut sink)
            .await
            .unwrap(),
        "bonjour"
    );
    assert_eq!(deltas, "bonjour");
    use base64::Engine;
    let image_path = root.path().join("selection.png");
    let pixels = base64::engine::general_purpose::STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAusB9Y9ZrNQAAAAASUVORK5CYII=").unwrap();
    std::fs::write(&image_path, pixels).unwrap();
    settings["settings"]["screenshotTranslation"]["ocrMode"] = json!("cloud_vision");
    std::fs::write(root.path().join("settings.json"), settings.to_string()).unwrap();
    let mut image_deltas = String::new();
    let mut finishes = 0;
    let translated = native_translation::translate_image(root.path(), &image_path, &mut |part| {
        match part {
            kivio_model_runtime::StreamPart::TextDelta { delta } => { assert_eq!(finishes, 0); image_deltas.push_str(&delta); }
            kivio_model_runtime::StreamPart::Finish { .. } => finishes += 1,
            _ => {}
        }
        Ok(())
    }).await.unwrap();
    assert_eq!(translated.original, "原文");
    assert_eq!(translated.translated, "译文");
    assert_eq!(image_deltas, "译文");
    assert_eq!(finishes, 1);
    let bodies = server.await.unwrap();
    assert!(bodies[4]["messages"].to_string().contains("data:image/png;base64,"));
    assert_eq!(bodies[3]["stream"], true);
    assert_eq!(bodies[2]["model"], "first");
    assert!(
        bodies[2]["messages"]
            .to_string()
            .contains("SELECTED=selected words;TARGET=French")
    );
    assert_eq!(bodies[0]["model"], "first");
    assert_eq!(bodies[1]["model"], "second");
    assert!(bodies[0]["messages"].to_string().contains("TARGET=English"));
    assert!(
        bodies[1]["messages"]
            .to_string()
            .contains("TARGET=Japanese")
    );
    let stats = kivio_model_runtime::usage_ledger::stats_in_directories(
        &[root.path().join("usage")],
        kivio_model_runtime::usage_ledger::UsageStatsQuery {
            range: "all".into(),
            ..Default::default()
        },
    );
    assert_eq!(stats.summary.total_requests, 5);
    assert!(
        stats
            .logs
            .iter()
            .filter(|record| record.source == "translator" && record.operation == "translate_text")
            .count()
            == 2
    );
}
