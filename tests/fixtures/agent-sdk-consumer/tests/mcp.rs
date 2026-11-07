#[path = "support/mcp_wire.rs"]
mod mcp_wire;
mod support;

use kivio_agent_sdk::{
    agent::{
        content::AgentToolStatus, execute::ToolProgress, AgentSession, AgentSessionConfig,
        ToolExecutionContext,
    },
    mcp::{McpClient, McpClientConfig, McpTransport},
    tools::{ToolAuthorization, ToolDefinition, ToolError, ToolFuture, ToolRegistry},
};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    future::pending,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    task::{JoinHandle, JoinSet},
};

const LIMIT: Duration = Duration::from_secs(8);

struct HttpServer {
    url: String,
    deleted: Arc<std::sync::atomic::AtomicBool>,
    task: JoinHandle<()>,
}

impl Drop for HttpServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl HttpServer {
    async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/mcp", listener.local_addr().unwrap());
        let state = Arc::new(Mutex::new(mcp_wire::WireState::default()));
        let deleted = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let seen_delete = deleted.clone();
        let task = tokio::spawn(async move {
            let mut clients = JoinSet::new();
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let (socket, _) = accepted.unwrap();
                        let state = state.clone();
                        let deleted = seen_delete.clone();
                        clients.spawn(async move { serve_http(socket, state, deleted).await; });
                    }
                    Some(result) = clients.join_next() => { result.unwrap(); }
                }
            }
        });
        Self { url, deleted, task }
    }
}

async fn serve_http(
    mut socket: TcpStream,
    state: Arc<Mutex<mcp_wire::WireState>>,
    deleted: Arc<std::sync::atomic::AtomicBool>,
) {
    let mut bytes = Vec::new();
    let end = loop {
        let mut chunk = [0; 4096];
        let count = socket.read(&mut chunk).await.unwrap();
        if count == 0 {
            return;
        }
        bytes.extend_from_slice(&chunk[..count]);
        if let Some(position) = bytes.windows(4).position(|value| value == b"\r\n\r\n") {
            break position + 4;
        }
    };
    let headers = String::from_utf8(bytes[..end].to_vec()).unwrap();
    assert!(headers
        .lines()
        .any(|line| line.eq_ignore_ascii_case("authorization: Bearer fixture-secret")));
    let method = headers.split_whitespace().next().unwrap();
    let length = headers
        .lines()
        .find_map(|line| {
            let (key, value) = line.split_once(':')?;
            key.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().unwrap())
        })
        .unwrap_or(0);
    while bytes.len() < end + length {
        let mut chunk = [0; 4096];
        let count = socket.read(&mut chunk).await.unwrap();
        if count == 0 {
            return;
        }
        bytes.extend_from_slice(&chunk[..count]);
    }
    let (status, content_type, body) = match method {
        "DELETE" => {
            deleted.store(true, std::sync::atomic::Ordering::SeqCst);
            (200, "application/json", String::new())
        }
        "GET" => (405, "application/json", String::new()),
        "POST" => {
            let request: Value = serde_json::from_slice(&bytes[end..end + length]).unwrap();
            let is_call = request["method"] == "tools/call";
            let held = is_call && request["params"]["name"] == "wait";
            let lost = is_call && request["params"]["name"] == "lost";
            let responses = state.lock().unwrap().receive(request);
            if lost {
                return;
            }
            if held {
                let mut byte = [0];
                let _ = tokio::time::timeout(LIMIT, socket.read(&mut byte)).await;
                return;
            }
            if responses.is_empty() {
                (202, "application/json", String::new())
            } else if responses.len() == 1 {
                (200, "application/json", responses[0].to_string())
            } else {
                (
                    200,
                    "text/event-stream",
                    responses
                        .iter()
                        .map(|value| format!("event: message\ndata: {value}\n\n"))
                        .collect(),
                )
            }
        }
        other => panic!("unexpected HTTP method {other}"),
    };
    let response = format!("HTTP/1.1 {status} Test\r\nContent-Type: {content_type}\r\nMcp-Session-Id: consumer-session\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
    let _ = socket.write_all(response.as_bytes()).await;
}

struct Fixture {
    client: Arc<McpClient>,
    http: Option<HttpServer>,
}
impl Fixture {
    async fn start(http: bool) -> Self {
        let server = if http {
            Some(HttpServer::start().await)
        } else {
            None
        };
        let transport = match &server {
            Some(server) => McpTransport::StreamableHttp {
                url: server.url.clone(),
                headers: HashMap::from([("Authorization".into(), "Bearer fixture-secret".into())]),
            },
            None => McpTransport::Stdio {
                command: env!("CARGO_BIN_EXE_mcp_fixture").into(),
                args: vec![],
                cwd: None,
                env: HashMap::new(),
            },
        };
        let client = tokio::time::timeout(
            LIMIT,
            McpClient::connect(
                McpClientConfig { transport },
                reqwest::Client::builder().no_proxy().build().unwrap(),
            ),
        )
        .await
        .unwrap()
        .unwrap();
        Self {
            client: Arc::new(client),
            http: server,
        }
    }
    async fn close(self) {
        let pid = self.client.child_pid();
        tokio::time::timeout(LIMIT, self.client.close())
            .await
            .unwrap()
            .unwrap();
        assert!(self.client.is_closed());
        if let Some(server) = &self.http {
            assert!(server.deleted.load(std::sync::atomic::Ordering::SeqCst));
        }
        if let Some(pid) = pid {
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                let output = std::process::Command::new("tasklist")
                    .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
                    .creation_flags(0x08000000)
                    .output()
                    .unwrap();
                assert!(output.status.success());
                assert!(
                    !String::from_utf8_lossy(&output.stdout).contains(&format!("\"{pid}\"")),
                    "MCP process remains after close"
                );
            }
            #[cfg(unix)]
            assert!(
                !std::process::Command::new("kill")
                    .args(["-0", &pid.to_string()])
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status()
                    .unwrap()
                    .success(),
                "MCP process remains after close"
            );
        }
    }
}

async fn stats(client: &McpClient) -> Value {
    client
        .call_tool("stats", json!({}), LIMIT, pending())
        .await
        .unwrap_or_else(|error| panic!("{}", error.message))
        .structured_content
        .unwrap()
}

struct Authorization(bool);
impl ToolAuthorization for Authorization {
    fn authorize<'a>(
        &'a self,
        _: &'a ToolProgress<'a>,
        _: &'a ToolExecutionContext<'a>,
        _: &'a ToolDefinition,
        _: &'a Value,
    ) -> ToolFuture<'a, Result<(), ToolError>> {
        Box::pin(async move {
            if self.0 {
                Ok(())
            } else {
                Err(ToolError::Execution("host denied MCP".into()))
            }
        })
    }
}

async fn registry(client: &Arc<McpClient>, allowed: bool) -> ToolRegistry {
    let mut tools = ToolRegistry::new();
    client
        .register_tools(
            &mut tools,
            "server",
            Arc::new(Authorization(allowed)),
            LIMIT,
        )
        .await
        .unwrap();
    tools
}

fn session(server: &support::Server, tools: ToolRegistry) -> AgentSession {
    let mut config = AgentSessionConfig::new(Arc::new(server.provider()), "fixture-model");
    config.tools = tools;
    config.options.thinking_enabled = false;
    AgentSession::new(config)
}

#[tokio::test]
async fn both_transports_discover_pages_register_authorize_and_preserve_results() {
    for http in [false, true] {
        let fixture = Fixture::start(http).await;
        assert_eq!(fixture.client.list_tools().await.unwrap().len(), 5);
        assert_eq!(stats(&fixture.client).await["lists"], 2);
        let tools = registry(&fixture.client, true).await;
        let echo = tools
            .definitions()
            .into_iter()
            .find(|tool| tool.model.name.ends_with("echo"))
            .unwrap();
        let model = support::Server::start(vec![
            support::tool(&echo.model.name, r#"{"text":"hello"}"#),
            support::answer("done"),
            support::tool(&echo.model.name, r#"{"text":"error"}"#),
            support::answer("handled"),
        ])
        .await;
        let session = session(&model, tools);
        let first = tokio::time::timeout(LIMIT, session.prompt("echo"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(first.tool_records[0].status, AgentToolStatus::Success);
        assert_eq!(first.tool_records[0].artifacts.len(), 1);
        assert_eq!(
            first.tool_records[0].structured_content,
            Some(json!({"echo":"hello"}))
        );
        let second = tokio::time::timeout(LIMIT, session.prompt("return error"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(second.tool_records[0].status, AgentToolStatus::Error);
        assert_eq!(second.tool_records[0].artifacts.len(), 1);
        fixture.close().await;
    }
}

#[tokio::test]
async fn annotations_never_authorize_and_invalid_dynamic_arguments_never_dispatch() {
    for http in [false, true] {
        let fixture = Fixture::start(http).await;
        for (allow, arguments, expected) in [
            (false, r#"{"text":"denied"}"#, AgentToolStatus::Skipped),
            (true, r#"{"text":42}"#, AgentToolStatus::Error),
        ] {
            let tools = registry(&fixture.client, allow).await;
            let name = tools
                .definitions()
                .into_iter()
                .find(|tool| tool.model.name.ends_with("echo"))
                .unwrap()
                .model
                .name;
            let model = support::Server::start(vec![
                support::tool(&name, arguments),
                support::answer("rejected"),
            ])
            .await;
            let result = session(&model, tools).prompt("try echo").await.unwrap();
            assert_eq!(result.tool_records[0].status, expected);
        }
        assert!(stats(&fixture.client).await["calls"]
            .as_array()
            .unwrap()
            .iter()
            .all(|call| call["name"] != "echo"));
        fixture.close().await;
    }
}

#[tokio::test]
async fn notifications_refresh_next_catalog_and_registration_is_atomic() {
    for http in [false, true] {
        let fixture = Fixture::start(http).await;
        let initial = registry(&fixture.client, true).await;
        fixture
            .client
            .call_tool("refresh", json!({"during_page":true}), LIMIT, pending())
            .await
            .unwrap_or_else(|error| panic!("{}", error.message));
        let refreshed = registry(&fixture.client, true).await;
        assert_eq!(initial.definitions().len(), 5);
        assert_eq!(refreshed.definitions().len(), 6);
        assert_eq!(stats(&fixture.client).await["lists"], 4);
        assert_eq!(fixture.client.list_tools().await.unwrap().len(), 6);
        assert_eq!(stats(&fixture.client).await["lists"], 6);
        let mut duplicate = refreshed.snapshot();
        assert!(fixture
            .client
            .register_tools(
                &mut duplicate,
                "server",
                Arc::new(Authorization(true)),
                LIMIT
            )
            .await
            .is_err());
        assert_eq!(duplicate.definitions().len(), 6);
        fixture
            .client
            .register_tools(
                &mut duplicate,
                "other",
                Arc::new(Authorization(true)),
                LIMIT,
            )
            .await
            .unwrap();
        assert_eq!(duplicate.definitions().len(), 12);
        fixture.close().await;
    }
}

#[tokio::test]
async fn cancellation_and_timeout_notify_without_closing_shared_connection_or_replay() {
    for http in [false, true] {
        let fixture = Fixture::start(http).await;
        let cancelled = fixture
            .client
            .call_tool("wait", json!({}), LIMIT, std::future::ready(()))
            .await
            .err()
            .unwrap();
        assert!(cancelled.replay_safe);
        let (cancel, cancelled) = tokio::sync::oneshot::channel();
        let client = fixture.client.clone();
        let waiting = tokio::spawn(async move {
            client
                .call_tool("wait", json!({}), LIMIT, async {
                    let _ = cancelled.await;
                })
                .await
        });
        tokio::time::timeout(LIMIT, async {
            loop {
                if stats(&fixture.client).await["calls"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|call| call["name"] == "wait")
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let other = fixture
            .client
            .call_tool("echo", json!({"text":"parallel"}), LIMIT, pending())
            .await
            .unwrap_or_else(|error| panic!("{}", error.message));
        assert!(other.content.contains("parallel"));
        cancel.send(()).unwrap();
        let failure = tokio::time::timeout(LIMIT, waiting)
            .await
            .unwrap()
            .unwrap()
            .err()
            .unwrap();
        assert!(!failure.replay_safe);
        let failure = fixture
            .client
            .call_tool("wait", json!({}), Duration::from_millis(50), pending())
            .await
            .err()
            .unwrap();
        assert!(!failure.replay_safe);
        let observed = stats(&fixture.client).await;
        assert_eq!(observed["cancelled"].as_array().unwrap().len(), 2);
        assert_eq!(
            observed["calls"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|call| call["name"] == "wait")
                .count(),
            2
        );
        fixture.close().await;
    }
}

#[tokio::test]
async fn lost_response_is_unknown_and_never_automatically_replayed() {
    let fixture = Fixture::start(true).await;
    let failure = fixture
        .client
        .call_tool("lost", json!({}), LIMIT, pending())
        .await
        .err()
        .unwrap();
    assert!(!failure.replay_safe);
    let observed = stats(&fixture.client).await;
    assert_eq!(
        observed["calls"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|call| call["name"] == "lost")
            .count(),
        1
    );
    fixture.close().await;

    let fixture = Fixture::start(false).await;
    let failure = fixture
        .client
        .call_tool("lost", json!({}), LIMIT, pending())
        .await
        .err()
        .unwrap();
    assert!(!failure.replay_safe);
    tokio::time::timeout(LIMIT, async {
        while !fixture.client.is_closed() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(fixture.client.cached_tools().is_none());
    fixture.close().await;
}

#[tokio::test]
async fn session_abort_waits_for_mcp_cancellation_and_can_continue() {
    for http in [false, true] {
        let fixture = Fixture::start(http).await;
        let tools = registry(&fixture.client, true).await;
        let name = tools
            .definitions()
            .into_iter()
            .find(|tool| tool.model.name.ends_with("wait"))
            .unwrap()
            .model
            .name;
        let model = support::Server::start(vec![
            support::tool(&name, "{}"),
            support::answer("continued"),
        ])
        .await;
        let session = session(&model, tools);
        let run = session.prompt("wait for cancellation");
        tokio::time::timeout(LIMIT, async {
            loop {
                if stats(&fixture.client).await["calls"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|call| call["name"] == "wait")
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        session.abort();
        tokio::time::timeout(LIMIT, session.wait_inactive())
            .await
            .unwrap();
        let _ = run.await;
        assert_eq!(
            stats(&fixture.client).await["cancelled"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            session.prompt("continue").await.unwrap().content,
            "continued"
        );
        fixture.close().await;
    }
}

#[tokio::test]
async fn concurrent_close_drains_an_active_request_before_all_waiters_return() {
    for http in [false, true] {
        let fixture = Fixture::start(http).await;
        let client = fixture.client.clone();
        let waiting =
            tokio::spawn(
                async move { client.call_tool("wait", json!({}), LIMIT, pending()).await },
            );
        tokio::time::timeout(LIMIT, async {
            loop {
                if stats(&fixture.client).await["calls"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|call| call["name"] == "wait")
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let (first, second) = tokio::time::timeout(LIMIT, async {
            tokio::join!(fixture.client.close(), fixture.client.close())
        })
        .await
        .unwrap();
        first.unwrap();
        second.unwrap();
        assert!(waiting.await.unwrap().is_err());
        assert!(fixture.client.cached_tools().is_none());
        fixture.close().await;
    }
}
