use kivio_model_runtime::{self as runtime, agent::{ToolExecutor, ToolExecutionContext}, native_tool_runtime::{NativeToolExecutor, NativeRunTools}};
use serde_json::{json, Value};
use std::{io::{Read, Write}, net::TcpListener, time::Duration};

#[tokio::test]
async fn native_video_tool_uses_selected_model_and_reuses_durable_observations() {
    let root = tempfile::tempdir().unwrap();
    let catalog = root.path().join("conversations");
    let attachments = catalog.join("conv_video_test_attachments");
    std::fs::create_dir_all(&attachments).unwrap();
    let clip = attachments.join("clip.mp4");
    std::fs::write(&clip, b"synthetic-video-payload").unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    listener.set_nonblocking(true).unwrap();
    let server = std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        let mut socket = loop {
            match listener.accept() {
                Ok((socket, _)) => break socket,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock && std::time::Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
                Err(e) => panic!("video request did not arrive: {e}"),
            }
        };
        socket.set_nonblocking(false).unwrap();
        socket.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        let mut bytes = Vec::new();
        let body = loop {
            let mut chunk = [0; 8192];
            let n = socket.read(&mut chunk).unwrap();
            assert!(n > 0);
            bytes.extend_from_slice(&chunk[..n]);
            if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                let length: usize = String::from_utf8_lossy(&bytes[..end]).lines().find_map(|l| l.to_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse().unwrap())).unwrap();
                if bytes.len() >= end + 4 + length { break serde_json::from_slice::<Value>(&bytes[end+4..end+4+length]).unwrap(); }
            }
        };
        let response = format!("data: {}\n\ndata: {}\n\ndata: [DONE]\n\n", json!({"id":"fixture", "choices":[{"index":0,"delta":{"content":"The lamp turns off."},"finish_reason":null}]}), json!({"id":"fixture","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}));
        write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",response.len(),response).unwrap();
        body
    });
    let provider = |id: &str, video: bool| json!({"id":id,"name":id,"baseUrl":format!("http://{address}/v1"),"enabled":true,"request":{"useSystemProxy":false},"apiFormat":"openai_chat","enabledModels":[if video {"video-model"} else {"fixture-model"}],"modelOverrides":{if video {"video-model"} else {"fixture-model"}:{"capabilities":{"videoInput":video}}}});
    let mut document = json!({"providers":[provider("main",false),provider("auxiliary",true)],"defaultModels":{"videoAnalysis":{"providerId":"auxiliary","model":"video-model"}},"videoAnalysisEnabled":true});
    std::fs::write(catalog.join("providers.json"), serde_json::to_vec(&document).unwrap()).unwrap();
    std::fs::write(root.path().join("settings.json"), serde_json::to_vec(&json!({"settings":{"retryEnabled":false,"chatTools":{"skillScanPaths":[],"nativeTools":{"workingDirectory":root.path()}}}})).unwrap()).unwrap();
    let mut conversation: runtime::conversation_data::Conversation = serde_json::from_value(json!({"id":"conv_video_test","title":"Video test","provider_id":"main","model":"fixture-model","messages":[{"id":"u1","role":"user","content":"Explain the ending","timestamp":1,"attachments":[{"id":"clip","type":"video","name":"clip.mp4","path":clip}]}],"created_at":1,"updated_at":1})).unwrap();
    let store = runtime::conversation_files::ConversationStore::new(catalog.clone());
    conversation = store.create(conversation).unwrap();
    let executor = NativeToolExecutor::new(root.path().into());
    let settings = runtime::native_preferences::load_runtime(root.path()).unwrap();
    let prepared = executor.prepare(&settings, &conversation, true).await.unwrap();
    let tool = prepared.tools.iter().find(|t| t.name == "mixer_video_analysis").expect("Chat mode must expose the selected video tool").clone();
    assert!(!prepared.tools.iter().any(|t| t.name == "bash" || t.name == "write"));
    let run = NativeRunTools { executor: &executor, video: tokio::sync::Mutex::new(prepared.video_analysis) };
    let context = ToolExecutionContext { conversation_id:"conv_video_test", tool_conversation_id:"conv_video_test",run_id:"run", message_id:"answer", generation:1, round:1, depth:0, tool_call_id:"call" };
    let result = run.call(&context, &tool, json!({"question":"Explain the ending"}), None).await.unwrap();
    assert!(result.content.contains("The lamp turns off."));
    let body = server.join().unwrap();
    assert_eq!(body["model"],"video-model");
    assert!(body["messages"].to_string().contains("data:video/mp4;base64,"));
    // File removal makes an accidental second inference/read fail, even without a live server.
    std::fs::remove_file(&clip).unwrap();
    assert_eq!(run.call(&context,&tool,json!({"question":"Why?"}),None).await.unwrap().content, result.content);
    conversation.messages.push(serde_json::from_value(json!({"id":"answer","role":"assistant","content":"A lamp.","timestamp":2,"tool_calls":[{"id":"call","name":"mixer_video_analysis","source":"mixer","arguments":"{}","status":"success","round":1,"structured_content":result.structured_content}]})).unwrap());
    let revision = conversation.revision;
    store.save(conversation,revision).unwrap();
    let reopened = store.load("conv_video_test").unwrap();
    let prepared = executor.prepare(&settings,&reopened,true).await.unwrap();
    assert!(prepared.video_reports.join("\n").contains("The lamp turns off."));
    let resumed = NativeRunTools { executor:&executor,video:tokio::sync::Mutex::new(prepared.video_analysis) };
    assert!(resumed.call(&context,&tool,json!({"question":"Why?"}),None).await.unwrap().content.contains("The lamp turns off."));
    let mut cleared = reopened.clone();
    cleared.context_state = serde_json::from_value(json!({"clear_boundaries":[{"id":"clear","source_until_message_id":"u1","created_at":3}]})).unwrap();
    let prepared = executor.prepare(&settings, &cleared, true).await.unwrap();
    assert!(prepared.video_reports.is_empty());
    assert!(!prepared.tools.iter().any(|t| t.name == "mixer_video_analysis"));
    let mut replaced = reopened.clone();
    replaced.messages[0].attachments[0].path = "replacement.mp4".into();
    assert!(executor.prepare(&settings, &replaced, true).await.unwrap().video_reports.is_empty());
    document["videoAnalysisEnabled"] = json!(false);
    std::fs::write(catalog.join("providers.json"), serde_json::to_vec(&document).unwrap()).unwrap();
    assert!(resumed.call(&context,&tool,json!({"question":"Why?"}),None).await.unwrap_err().contains("disabled"));
    let disabled = runtime::native_preferences::load_runtime(root.path()).unwrap();
    let prepared = executor.prepare(&disabled,&reopened,true).await.unwrap();
    assert!(!prepared.tools.iter().any(|t| t.name == "mixer_video_analysis"));
    assert!(!prepared.video_reports.is_empty());
    document["videoAnalysisEnabled"] = json!(true);
    document["defaultModels"]["videoAnalysis"] = json!({});
    std::fs::write(catalog.join("providers.json"), serde_json::to_vec(&document).unwrap()).unwrap();
    let auto = runtime::native_preferences::load_runtime(root.path()).unwrap();
    assert_eq!(runtime::video_analysis::plan(&auto, &auto.providers[0], "fixture-model", &reopened).unwrap().model.unwrap().provider_id,"auxiliary");
    let direct = runtime::video_analysis::plan(&disabled,&auto.providers[1],"video-model",&reopened).unwrap();
    assert!(direct.send_video);
    assert!(direct.model.is_none());
    for broken in [json!([]), json!(null), json!({"videoAnalysisEnabled":"false"}), json!({"providers":{}})] {
        std::fs::write(catalog.join("providers.json"), serde_json::to_vec(&broken).unwrap()).unwrap();
        assert!(runtime::native_preferences::load_runtime(root.path()).is_err(), "damaged catalog must not fall back to stale legacy models");
    }
    executor.shutdown().await;
}
