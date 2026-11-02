use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};

use crate::{
    app_mcp::{Backend, ServerCredential, ToolOutput, MAX_BODY},
    chat::voice_orchestrator,
    mcp::types,
};

#[cfg(test)]
use crate::app_mcp::{accepts_json, same_secret, MAX_CALLS};
#[cfg(test)]
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
#[cfg(test)]
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
#[cfg(test)]
use tokio::net::TcpStream;

pub(super) type VoiceMcp = ServerCredential<AppBackend>;

#[derive(Clone)]
pub(super) struct AppBackend {
    app: AppHandle,
    session_id: String,
}

impl Backend for AppBackend {
    async fn call(&self, name: &str, arguments: &Value) -> Result<ToolOutput, String> {
        voice_orchestrator::call_tool(&self.app, name, arguments)
            .await
            .map(ToolOutput::from)
    }

    fn working(&self, count: usize) {
        let _ = self.app.emit(
            "voice-codex-event",
            json!({
                "sessionId": self.session_id,
                "type": "working",
                "count": count,
            }),
        );
    }
}

pub(super) async fn start(app: AppHandle, session_id: String) -> Result<VoiceMcp, String> {
    start_server(AppBackend { app, session_id }).await
}

fn tool_definitions() -> Vec<Value> {
    [
        types::native_kivio_list_agents_tool(),
        types::native_kivio_list_projects_tool(),
        types::native_kivio_list_conversations_tool(),
        types::native_kivio_read_conversation_tool(),
        types::native_kivio_create_conversation_tool(),
        types::native_kivio_send_message_tool(),
        types::native_kivio_interrupt_tool(),
    ]
    .into_iter()
    .map(|tool| {
        json!({
            "name": tool.name,
            "description": tool.description,
            "inputSchema": tool.input_schema,
        })
    })
    .collect()
}

async fn start_server<B: Backend>(backend: B) -> Result<ServerCredential<B>, String> {
    crate::app_mcp::start_server(
        tool_definitions(),
        "kivio-voice",
        MAX_BODY,
        "voice",
        "Voice tool timed out; its outcome may be unknown",
    )
    .await?
    .issue(backend)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::Notify;

    #[derive(Clone, Default)]
    struct TestBackend {
        events: Arc<Mutex<Vec<usize>>>,
        started: Arc<Notify>,
        release: Arc<Notify>,
    }

    impl Backend for TestBackend {
        async fn call(&self, _name: &str, arguments: &Value) -> Result<ToolOutput, String> {
            self.started.notify_one();
            self.release.notified().await;
            if arguments.get("fail") == Some(&json!(true)) {
                Err("tool failed".into())
            } else {
                Ok(ToolOutput::from("tool result".to_string()))
            }
        }

        fn working(&self, count: usize) {
            self.events.lock().unwrap().push(count);
        }
    }

    fn client() -> reqwest::Client {
        reqwest::Client::builder().no_proxy().build().unwrap()
    }

    fn post(
        client: &reqwest::Client,
        server: &ServerCredential<TestBackend>,
        message: Value,
    ) -> reqwest::RequestBuilder {
        client
            .post(&server.url)
            .header(header::AUTHORIZATION, &server.authorization)
            .header(header::ACCEPT, "application/json, text/event-stream")
            .json(&message)
    }

    async fn initialize(
        client: &reqwest::Client,
        server: &ServerCredential<TestBackend>,
        requested: &str,
    ) -> Value {
        let response = post(client, server, json!({
            "jsonrpc":"2.0", "id":1, "method":"initialize",
            "params":{"protocolVersion":requested,"capabilities":{},"clientInfo":{"name":"test","version":"1"}}
        })).send().await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response.json().await.unwrap();
        let notification = post(
            client,
            server,
            json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        )
        .send()
        .await
        .unwrap();
        assert_eq!(notification.status(), StatusCode::ACCEPTED);
        assert!(notification.bytes().await.unwrap().is_empty());
        body
    }

    #[tokio::test]
    async fn handshake_lists_the_exact_voice_tools_and_negotiates_versions() {
        let server = start_server(TestBackend::default()).await.unwrap();
        let client = client();
        let init = initialize(&client, &server, "2025-06-18").await;
        assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
        let response = post(
            &client,
            &server,
            json!({"jsonrpc":"2.0","id":"list","method":"tools/list"}),
        )
        .header("mcp-protocol-version", "2025-06-18")
        .send()
        .await
        .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let response: Value = response.json().await.unwrap();
        assert_eq!(response["id"], "list");
        assert_eq!(response["result"]["tools"], json!(tool_definitions()));
        assert_eq!(response["result"]["tools"].as_array().unwrap().len(), 7);

        let init = initialize(&client, &server, "2099-01-01").await;
        assert_eq!(init["result"]["protocolVersion"], "2025-11-25");
        let bad_version = post(
            &client,
            &server,
            json!({"jsonrpc":"2.0","id":2,"method":"ping"}),
        )
        .header("mcp-protocol-version", "2099-01-01")
        .send()
        .await
        .unwrap();
        assert_eq!(bad_version.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn rejects_hostile_duplicate_and_missing_security_headers() {
        let server = start_server(TestBackend::default()).await.unwrap();
        let client = client();
        let request = json!({"jsonrpc":"2.0","id":1,"method":"ping"});
        assert_eq!(
            client
                .post(&server.url)
                .json(&request)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            post(&client, &server, request.clone())
                .header(header::HOST, "evil.example")
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::FORBIDDEN
        );
        for origin in ["null", "https://evil.example", "http://localhost"] {
            assert_eq!(
                post(&client, &server, request.clone())
                    .header(header::ORIGIN, origin)
                    .send()
                    .await
                    .unwrap()
                    .status(),
                StatusCode::FORBIDDEN
            );
        }
        let origin = server.url.strip_suffix("/mcp").unwrap();
        let host = origin.strip_prefix("http://").unwrap();
        let duplicate_host = post(&client, &server, request.clone())
            .header(header::HOST, host)
            .header(header::HOST, host)
            .send()
            .await;
        assert!(duplicate_host
            .map(|response| !response.status().is_success())
            .unwrap_or(true));
        assert_eq!(
            post(&client, &server, request.clone())
                .header(header::ORIGIN, origin)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::OK
        );
        assert_eq!(
            post(&client, &server, request.clone())
                .header(header::ORIGIN, origin)
                .header(header::ORIGIN, origin)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            post(&client, &server, request.clone())
                .header(header::AUTHORIZATION, &server.authorization)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            client
                .get(&server.url)
                .header(header::AUTHORIZATION, &server.authorization)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::METHOD_NOT_ALLOWED
        );
        assert_eq!(
            post(&client, &server, request)
                .header(header::ACCEPT, "text/event-stream")
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::OK
        );
        let sse_only = client
            .post(&server.url)
            .header(header::AUTHORIZATION, &server.authorization)
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::ACCEPT, "text/event-stream")
            .body(r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#)
            .send()
            .await
            .unwrap();
        assert_eq!(sse_only.status(), StatusCode::NOT_ACCEPTABLE);
        let mut headers = HeaderMap::new();
        headers.insert(
            header::ACCEPT,
            HeaderValue::from_static("*/*;q=1, application/json;q=0"),
        );
        assert!(!accepts_json(&headers));
    }

    #[tokio::test]
    async fn rpc_errors_body_limit_and_tool_results_are_observable_over_http() {
        let backend = TestBackend::default();
        let server = start_server(backend.clone()).await.unwrap();
        let client = client();
        initialize(&client, &server, "2025-11-25").await;
        let bad = client
            .post(&server.url)
            .header(header::AUTHORIZATION, &server.authorization)
            .header(header::CONTENT_TYPE, "application/json")
            .body("{")
            .send()
            .await
            .unwrap();
        assert_eq!(bad.status(), StatusCode::BAD_REQUEST);
        assert_eq!(bad.json::<Value>().await.unwrap()["error"]["code"], -32700);
        let large = client
            .post(&server.url)
            .header(header::AUTHORIZATION, &server.authorization)
            .header(header::CONTENT_TYPE, "application/json")
            .body(" ".repeat(MAX_BODY + 1))
            .send()
            .await
            .unwrap();
        assert_eq!(large.status(), StatusCode::PAYLOAD_TOO_LARGE);
        let unknown = post(
            &client,
            &server,
            json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"unknown"}}),
        )
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
        assert_eq!(unknown["error"]["code"], -32602);

        backend.release.notify_one();
        let result = post(
            &client,
            &server,
            json!({"jsonrpc":"2.0","id":3,"method":"tools/call",
            "params":{"name":"kivio_list_agents","arguments":{"fail":true}}}),
        )
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
        assert_eq!(result["result"]["isError"], true);
        assert_eq!(result["result"]["content"][0]["text"], "tool failed");
        assert_eq!(*backend.events.lock().unwrap(), vec![1, 0]);
    }

    #[tokio::test]
    async fn credentials_are_session_scoped_and_drop_closes_idle_sockets() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let first = start_server(TestBackend::default()).await.unwrap();
        let second = start_server(TestBackend::default()).await.unwrap();
        assert!(first.url.starts_with("http://127.0.0.1:"));
        assert!(!same_secret(&first.authorization, &second.authorization));
        let response = client()
            .post(&second.url)
            .header(header::AUTHORIZATION, &first.authorization)
            .json(&json!({"jsonrpc":"2.0","id":1,"method":"ping"}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        let address = first
            .url
            .trim_start_matches("http://")
            .trim_end_matches("/mcp");
        let mut socket = TcpStream::connect(address).await.unwrap();
        socket
            .write_all(b"POST /mcp HTTP/1.1\r\nHost: ")
            .await
            .unwrap();
        drop(first);
        let mut buffer = [0u8; 1];
        let read = tokio::time::timeout(Duration::from_secs(3), socket.read(&mut buffer))
            .await
            .unwrap();
        assert!(matches!(read, Ok(0) | Err(_)));
    }

    #[tokio::test]
    async fn concurrent_calls_are_bounded_and_cancellations_update_counts() {
        let backend = TestBackend::default();
        let server = start_server(backend.clone()).await.unwrap();
        let client = client();
        initialize(&client, &server, "2025-06-18").await;
        let mut tasks = Vec::new();
        for id in 0..MAX_CALLS {
            let pending = post(
                &client,
                &server,
                json!({"jsonrpc":"2.0","id":id,"method":"tools/call",
                "params":{"name":"kivio_list_agents","arguments":{}}}),
            );
            tasks.push(tokio::spawn(async move { pending.send().await }));
            tokio::time::timeout(Duration::from_secs(3), backend.started.notified())
                .await
                .unwrap();
        }
        let denied = post(
            &client,
            &server,
            json!({"jsonrpc":"2.0","id":99,"method":"tools/call",
            "params":{"name":"kivio_list_agents","arguments":{}}}),
        )
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
        assert_eq!(denied["error"]["code"], -32000);
        for (id, task) in tasks.into_iter().enumerate() {
            let response = post(
                &client,
                &server,
                json!({"jsonrpc":"2.0","method":"notifications/cancelled",
                "params":{"requestId":id}}),
            )
            .send()
            .await
            .unwrap();
            assert_eq!(response.status(), StatusCode::ACCEPTED);
            assert_eq!(task.await.unwrap().unwrap().status(), StatusCode::ACCEPTED);
        }
        let expected: Vec<usize> = (1..=MAX_CALLS).chain((0..MAX_CALLS).rev()).collect();
        assert_eq!(*backend.events.lock().unwrap(), expected);
    }

    #[tokio::test]
    async fn sdk_client_initializes_lists_and_calls_over_streamable_http() {
        use rmcp::{
            model::CallToolRequestParams,
            transport::{
                streamable_http_client::StreamableHttpClientTransportConfig,
                StreamableHttpClientTransport,
            },
            ClientLifecycleMode, ClientServiceExt,
        };

        let backend = TestBackend::default();
        let server = start_server(backend.clone()).await.unwrap();
        let mut config = StreamableHttpClientTransportConfig::with_uri(server.url.clone());
        config.custom_headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_str(&server.authorization).unwrap(),
        );
        let transport = StreamableHttpClientTransport::with_client(client(), config);
        let service = crate::mcp::conn::KivioClientHandler::default()
            .serve_with_lifecycle(transport, ClientLifecycleMode::Initialize)
            .await
            .unwrap();
        let tools = service.list_all_tools().await.unwrap();
        assert_eq!(tools.len(), 7);
        assert!(tools.iter().any(|tool| tool.name == "kivio_list_agents"));
        backend.release.notify_one();
        let result = service
            .call_tool(CallToolRequestParams::new("kivio_list_agents".to_string()))
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(result).unwrap()["content"][0]["text"],
            "tool result"
        );
    }

    #[tokio::test]
    async fn cancellation_and_owner_drop_clear_pending_work() {
        let backend = TestBackend::default();
        let server = start_server(backend.clone()).await.unwrap();
        let client = client();
        initialize(&client, &server, "2025-06-18").await;
        let pending = post(
            &client,
            &server,
            json!({"jsonrpc":"2.0","id":42,"method":"tools/call",
            "params":{"name":"kivio_list_agents","arguments":{}}}),
        );
        let task = tokio::spawn(async move { pending.send().await });
        tokio::time::timeout(Duration::from_secs(3), backend.started.notified())
            .await
            .unwrap();
        let cancel = post(
            &client,
            &server,
            json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":42}}),
        )
        .send()
        .await
        .unwrap();
        assert_eq!(cancel.status(), StatusCode::ACCEPTED);
        assert_eq!(task.await.unwrap().unwrap().status(), StatusCode::ACCEPTED);
        assert_eq!(*backend.events.lock().unwrap(), vec![1, 0]);

        let pending = post(
            &client,
            &server,
            json!({"jsonrpc":"2.0","id":43,"method":"tools/call",
            "params":{"name":"kivio_list_projects","arguments":{}}}),
        );
        let task = tokio::spawn(async move { pending.send().await });
        tokio::time::timeout(Duration::from_secs(3), backend.started.notified())
            .await
            .unwrap();
        drop(server);
        let _ = tokio::time::timeout(Duration::from_secs(3), task)
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if *backend.events.lock().unwrap() == vec![1, 0, 1, 0] {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
    }
}
