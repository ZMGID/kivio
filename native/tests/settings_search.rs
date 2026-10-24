//! The native search host must execute the original HTTP and pooled MCP contracts.
use kivio_model_runtime::{
    mcp::NativeMcp,
    settings::{ChatMcpServer, LensWebSearchConfig, WebSearchProvider},
    web_search_host::NativeSearch,
};
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
#[tokio::test]
async fn native_search_sends_draft_configuration_and_returns_source_content() {
    let root = tempfile::tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        loop {
            let mut bytes = [0; 4096];
            let count = socket.read(&mut bytes).await.unwrap();
            assert!(count > 0);
            request.extend_from_slice(&bytes[..count]);
            if let Some(end) = request.windows(4).position(|b| b == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&request[..end]);
                let length: usize = headers
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|v| v.trim().parse().unwrap())
                    })
                    .unwrap();
                if request.len() >= end + 4 + length {
                    break;
                }
            }
        }
        let response = json!({"data":{"webPages":{"value":[{"name":"Fixture source","url":"https://example.test/article","summary":"Source content"}]}}}).to_string();
        socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}", response.len()).as_bytes()).await.unwrap();
        String::from_utf8(request).unwrap()
    });
    let config = LensWebSearchConfig {
        provider: WebSearchProvider::Bocha,
        bocha_base_url: format!("http://{address}"),
        bocha_api_key: "fixture-secret".into(),
        max_results: 3,
        ..Default::default()
    };
    let runtime = NativeSearch::new(root.path().into());
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        runtime.search(&config, "native search"),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].content, "Source content");
    assert_eq!(result[0].url, "https://example.test/article");
    let request = server.await.unwrap();
    assert!(request.starts_with("POST /v1/web-search "));
    assert!(
        request
            .to_lowercase()
            .contains("authorization: bearer fixture-secret")
    );
    let body: serde_json::Value =
        serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
    assert_eq!(body["query"], "native search");
    assert_eq!(body["count"], 3);
    runtime.shutdown().await;
}
#[tokio::test]
async fn native_mcp_reuses_session_and_shuts_down_child() {
    let root = tempfile::tempdir().unwrap();
    let script = root.path().join("server.py");
    let starts = root.path().join("starts");
    std::fs::write(&script, r#"import json, sys, os
with open(sys.argv[1], 'a') as f: f.write(str(os.getpid()) + '\n')
for line in sys.stdin:
 r=json.loads(line)
 if 'id' not in r: continue
 m=r['method']
 if m=='initialize': value={'protocolVersion':'2025-06-18','capabilities':{'tools':{}},'serverInfo':{'name':'fixture','version':'1'}}
 elif m=='tools/list': value={'tools':[{'name':'echo','description':'echo','inputSchema':{'type':'object'}}]}
 elif m=='tools/call': value={'content':[{'type':'text','text':r['params']['arguments']['text']}]}
 else: value={}
 print(json.dumps({'jsonrpc':'2.0','id':r['id'],'result':value}),flush=True)
"#).unwrap();
    let server = ChatMcpServer {
        id: "fixture".into(),
        name: "Fixture".into(),
        enabled: true,
        transport: "stdio".into(),
        command: "python3".into(),
        args: vec![
            "-u".into(),
            script.to_string_lossy().into(),
            starts.to_string_lossy().into(),
        ],
        ..Default::default()
    };
    let runtime = NativeMcp::new(root.path().into());
    assert_eq!(runtime.list(&server).await.unwrap()[0].name, "echo");
    for text in ["first", "second"] {
        assert_eq!(
            runtime
                .call(&server, "echo", json!({"text":text}))
                .await
                .unwrap()
                .content,
            text
        );
    }
    let started = std::fs::read_to_string(&starts).unwrap();
    assert_eq!(
        started.lines().count(),
        1,
        "calls must share the negotiated session"
    );
    runtime.shutdown().await;
    #[cfg(unix)]
    {
        let pid: i32 = started.trim().parse().unwrap();
        let dead = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while unsafe { libc::kill(pid, 0) } == 0 {
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await;
        assert!(dead.is_ok(), "shutdown must reap the MCP child");
    }
}

#[test]
fn oauth_refresh_persists_auth_without_overwriting_other_settings() {
    use kivio_model_runtime::{
        native_preferences::{self, Change},
    };
    let root = tempfile::tempdir().unwrap();
    let auth = json!({"kind":"oauth","accessToken":"fixture-old","refreshToken":"fixture-refresh","expiresAt":10});
    std::fs::write(root.path().join("settings.json"), json!({"futureEnvelope":7,"settings":{"futureSetting":true,"lens":{"webSearch":{"tinyfishMcpUrl":"https://fixture.test/mcp","tinyfishMcpAuth":auth}},"chatTools":{"servers":[{"id":"fixture","name":"Fixture","transport":"streamable_http","url":"https://fixture.test/mcp","auth":auth,"headers":{"Authorization":"Bearer fixture-old","X-Custom":"keep"},"futureServer":42}]}}}).to_string()).unwrap();
    let before = native_preferences::load(root.path())
        .unwrap()
        .lens
        .web_search;
    let mut authorized = before.clone();
    authorized.tinyfish_mcp_auth.as_mut().unwrap().access_token = "fixture-authorized".into();
    native_preferences::save(
        root.path(),
        Change::WebSearch {
            before: before.clone(),
            after: authorized.clone(),
        },
    )
    .unwrap();
    // A delayed browser result must not overwrite a later authorization.
    assert!(
        native_preferences::save(
            root.path(),
            Change::WebSearch {
                before,
                after: authorized
            }
        )
        .is_err()
    );
    let mut refreshed = native_preferences::load(root.path())
        .unwrap()
        .chat_tools
        .servers
        .remove(0);
    refreshed.auth.as_mut().unwrap().access_token = "fixture-new".into();
    refreshed
        .headers
        .insert("Authorization".into(), "Bearer fixture-new".into());
    native_preferences::save(root.path(), Change::RefreshedMcpAuth(refreshed.clone())).unwrap();
    let loaded = native_preferences::load(root.path()).unwrap();
    assert_eq!(
        loaded.chat_tools.servers[0]
            .auth
            .as_ref()
            .unwrap()
            .access_token,
        "fixture-new"
    );
    assert_eq!(
        loaded
            .lens
            .web_search
            .tinyfish_mcp_auth
            .as_ref()
            .unwrap()
            .access_token,
        "fixture-new"
    );
    let doc: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.path().join("settings.json")).unwrap()).unwrap();
    // A refresh started for an old endpoint must not attach its token to a newly edited endpoint.
    let mut changed_endpoint = doc.clone();
    changed_endpoint["settings"]["chatTools"]["servers"][0]["url"] = json!("https://other.test/mcp");
    changed_endpoint["settings"]["chatTools"]["servers"][0]["auth"] = serde_json::Value::Null;
    changed_endpoint["settings"]["chatTools"]["servers"][0]["headers"].as_object_mut().unwrap().remove("Authorization");
    std::fs::write(root.path().join("settings.json"), changed_endpoint.to_string()).unwrap();
    native_preferences::save(root.path(), Change::RefreshedMcpAuth(refreshed)).unwrap();
    assert!(native_preferences::load(root.path()).unwrap().chat_tools.servers[0].auth.is_none(), "old endpoint authorization was restored onto the edited server");
    assert_eq!(doc["futureEnvelope"], 7);
    assert_eq!(doc["settings"]["futureSetting"], true);
    assert_eq!(
        doc["settings"]["chatTools"]["servers"][0]["futureServer"],
        42
    );
    assert_eq!(
        doc["settings"]["chatTools"]["servers"][0]["headers"]["X-Custom"],
        "keep"
    );
}
