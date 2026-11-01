use std::{
    collections::HashMap,
    future::Future,
    io,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
    time::Duration,
};

use axum::{
    body::{to_bytes, Body},
    extract::State,
    http::{header, HeaderMap, HeaderValue, Method, Request, StatusCode, Version},
    response::{IntoResponse, Response},
    Json, Router,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use rand::RngCore;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    net::{TcpListener, TcpStream},
    sync::{oneshot, watch, OwnedSemaphorePermit, Semaphore},
    task::JoinHandle,
    time::{Instant, Sleep},
};

use crate::{chat::voice_orchestrator, mcp::types};

const MAX_BODY: usize = 256 * 1024;
const MAX_CONNECTIONS: usize = 32;
const MAX_CALLS: usize = 8;
const HEADER_TIMEOUT: Duration = Duration::from_secs(10);
const BODY_TIMEOUT: Duration = Duration::from_secs(10);
// The orchestrator can itself wait ten minutes for a delegated reply.
const TOOL_TIMEOUT: Duration = Duration::from_secs(620);
const CONNECTION_TIMEOUT: Duration = Duration::from_secs(650);
const VERSIONS: &[&str] = &["2025-06-18", "2025-11-25"];

pub(super) struct VoiceMcp {
    pub url: String,
    // This is the complete value of the Authorization header, not just the token.
    pub authorization: String,
    shutdown: watch::Sender<bool>,
    task: JoinHandle<()>,
}

impl Drop for VoiceMcp {
    fn drop(&mut self) {
        self.shutdown.send_replace(true);
        self.task.abort();
    }
}

trait Backend: Clone + Send + Sync + 'static {
    fn call(
        &self,
        name: &str,
        arguments: &Value,
    ) -> impl Future<Output = Result<String, String>> + Send;
    fn working(&self, count: usize);
}

#[derive(Clone)]
struct AppBackend {
    app: AppHandle,
    session_id: String,
}

impl Backend for AppBackend {
    async fn call(&self, name: &str, arguments: &Value) -> Result<String, String> {
        voice_orchestrator::call_tool(&self.app, name, arguments).await
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

struct Control {
    negotiated: Option<&'static str>,
    initialized: bool,
    calls: HashMap<String, Option<oneshot::Sender<()>>>,
}

struct ServerState<B> {
    backend: B,
    host: String,
    origin: String,
    authorization: String,
    tools: Vec<Value>,
    control: Mutex<Control>,
    call_slots: Arc<Semaphore>,
    shutdown: watch::Sender<bool>,
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

async fn start_server<B: Backend>(backend: B) -> Result<VoiceMcp, String> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|error| format!("Failed to bind voice MCP listener: {error}"))?;
    let address = listener
        .local_addr()
        .map_err(|error| format!("Failed to inspect voice MCP listener: {error}"))?;
    let host = address.to_string();
    let origin = format!("http://{host}");
    let mut secret = [0u8; 32];
    rand::rngs::OsRng
        .try_fill_bytes(&mut secret)
        .map_err(|_| "Failed to generate voice MCP credential".to_string())?;
    let authorization = format!("Bearer {}", URL_SAFE_NO_PAD.encode(secret));
    let (shutdown, _) = watch::channel(false);
    let state = Arc::new(ServerState {
        backend,
        host,
        origin: origin.clone(),
        authorization: authorization.clone(),
        tools: tool_definitions(),
        control: Mutex::new(Control {
            negotiated: None,
            initialized: false,
            calls: HashMap::new(),
        }),
        call_slots: Arc::new(Semaphore::new(MAX_CALLS)),
        shutdown: shutdown.clone(),
    });
    let listener = LimitedListener {
        listener,
        slots: Arc::new(Semaphore::new(MAX_CONNECTIONS)),
        shutdown: shutdown.subscribe(),
    };
    let router = Router::new().fallback(endpoint::<B>).with_state(state);
    let task = tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    Ok(VoiceMcp {
        url: format!("{origin}/mcp"),
        authorization,
        shutdown,
        task,
    })
}

async fn stopped(mut shutdown: watch::Receiver<bool>) {
    if !*shutdown.borrow() {
        let _ = shutdown.changed().await;
    }
}

// axum spawns connection tasks independently of its accept loop. Cap them and
// make their sockets close on owner drop, including incomplete HTTP headers.
struct LimitedListener {
    listener: TcpListener,
    slots: Arc<Semaphore>,
    shutdown: watch::Receiver<bool>,
}

impl axum::serve::Listener for LimitedListener {
    type Io = LimitedStream;
    type Addr = std::net::SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        let permit = self
            .slots
            .clone()
            .acquire_owned()
            .await
            .expect("connection semaphore is never closed");
        let (stream, address) = axum::serve::Listener::accept(&mut self.listener).await;
        let shutdown = self.shutdown.clone();
        (
            LimitedStream {
                stream,
                _permit: permit,
                shutdown: Box::pin(stopped(shutdown)),
                deadline: Box::pin(tokio::time::sleep(HEADER_TIMEOUT)),
                header_prefix: 0,
                headers_complete: false,
            },
            address,
        )
    }

    fn local_addr(&self) -> io::Result<Self::Addr> {
        self.listener.local_addr()
    }
}

struct LimitedStream {
    stream: TcpStream,
    _permit: OwnedSemaphorePermit,
    shutdown: Pin<Box<dyn Future<Output = ()> + Send>>,
    deadline: Pin<Box<Sleep>>,
    header_prefix: usize,
    headers_complete: bool,
}

impl LimitedStream {
    fn expired(&mut self, cx: &mut Context<'_>) -> bool {
        self.shutdown.as_mut().poll(cx).is_ready() || self.deadline.as_mut().poll(cx).is_ready()
    }
}

fn closed_socket() -> io::Error {
    io::Error::new(
        io::ErrorKind::ConnectionAborted,
        "voice MCP connection closed",
    )
}

impl AsyncRead for LimitedStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if self.expired(cx) {
            return Poll::Ready(Err(closed_socket()));
        }
        let before = buf.filled().len();
        let result = Pin::new(&mut self.stream).poll_read(cx, buf);
        if matches!(result, Poll::Ready(Ok(()))) && !self.headers_complete {
            for &byte in &buf.filled()[before..] {
                if byte == b"\r\n\r\n"[self.header_prefix] {
                    self.header_prefix += 1;
                    if self.header_prefix == 4 {
                        self.headers_complete = true;
                        self.deadline
                            .as_mut()
                            .reset(Instant::now() + CONNECTION_TIMEOUT);
                        break;
                    }
                } else {
                    self.header_prefix = usize::from(byte == b'\r');
                }
            }
        }
        result
    }
}

impl AsyncWrite for LimitedStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        data: &[u8],
    ) -> Poll<io::Result<usize>> {
        if self.expired(cx) {
            return Poll::Ready(Err(closed_socket()));
        }
        Pin::new(&mut self.stream).poll_write(cx, data)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if self.expired(cx) {
            return Poll::Ready(Err(closed_socket()));
        }
        Pin::new(&mut self.stream).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_shutdown(cx)
    }
}

fn one_header<'a>(headers: &'a HeaderMap, name: &'static str) -> Result<Option<&'a str>, ()> {
    let all = headers.get_all(name);
    let mut values = all.iter();
    let first = values.next();
    if values.next().is_some() {
        return Err(());
    }
    first
        .map(|value| value.to_str().map_err(|_| ()))
        .transpose()
}

fn same_secret(actual: &str, expected: &str) -> bool {
    actual.len() == expected.len()
        && actual
            .as_bytes()
            .iter()
            .zip(expected.as_bytes())
            .fold(0u8, |diff, (a, b)| diff | (a ^ b))
            == 0
}

fn authorize<B>(state: &ServerState<B>, request: &Request<Body>) -> Result<(), StatusCode> {
    let headers = request.headers();
    if one_header(headers, "host").ok().flatten() != Some(state.host.as_str())
        || request
            .uri()
            .authority()
            .is_some_and(|authority| authority.as_str() != state.host)
    {
        return Err(StatusCode::FORBIDDEN);
    }
    match one_header(headers, "origin") {
        Ok(None) => {}
        Ok(Some(origin)) if origin == state.origin => {}
        _ => return Err(StatusCode::FORBIDDEN),
    }
    if !one_header(headers, "authorization")
        .ok()
        .flatten()
        .is_some_and(|value| same_secret(value, &state.authorization))
    {
        return Err(StatusCode::UNAUTHORIZED);
    }
    match one_header(headers, "mcp-protocol-version") {
        Ok(None) => {}
        Ok(Some(version)) if VERSIONS.contains(&version) => {}
        _ => return Err(StatusCode::BAD_REQUEST),
    }
    Ok(())
}

fn accepts_json(headers: &HeaderMap) -> bool {
    let all = headers.get_all(header::ACCEPT);
    if all.iter().next().is_none() {
        return true;
    }
    let mut best: Option<(u8, f32)> = None;
    for item in all
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
    {
        let mut parts = item.split(';');
        let media = parts.next().unwrap_or("").trim();
        let specificity = if media.eq_ignore_ascii_case("application/json") {
            2
        } else if media.eq_ignore_ascii_case("application/*") {
            1
        } else if media == "*/*" {
            0
        } else {
            continue;
        };
        let mut quality = 1.0f32;
        for part in parts {
            if let Some((name, value)) = part.trim().split_once('=') {
                if name.trim().eq_ignore_ascii_case("q") {
                    quality = value.trim().parse().unwrap_or(0.0);
                }
            }
        }
        if !quality.is_finite() || !(0.0..=1.0).contains(&quality) {
            quality = 0.0;
        }
        if best.is_none_or(|(current, q)| {
            specificity > current || (specificity == current && quality > q)
        }) {
            best = Some((specificity, quality));
        }
    }
    best.is_some_and(|(_, quality)| quality > 0.0)
}

fn rpc_result(id: Value, result: Value) -> Response {
    Json(json!({"jsonrpc": "2.0", "id": id, "result": result})).into_response()
}

fn rpc_error(status: StatusCode, id: Value, code: i32, message: &str) -> Response {
    (
        status,
        Json(json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})),
    )
        .into_response()
}

fn valid_id(id: &Value) -> bool {
    id.is_string() || id.as_i64().is_some() || id.as_u64().is_some()
}

async fn endpoint<B: Backend>(
    State(state): State<Arc<ServerState<B>>>,
    request: Request<Body>,
) -> Response {
    let http_version = request.version();
    let mut response = handle_request(state, request).await;
    // The connection's deadline covers one complete HTTP/1 exchange, including
    // slow headers and bodies, rather than cutting short a later reused request.
    if http_version != Version::HTTP_2 {
        response
            .headers_mut()
            .insert(header::CONNECTION, HeaderValue::from_static("close"));
    }
    response
}

async fn handle_request<B: Backend>(
    state: Arc<ServerState<B>>,
    request: Request<Body>,
) -> Response {
    if let Err(status) = authorize(&state, &request) {
        let mut response = status.into_response();
        if status == StatusCode::UNAUTHORIZED {
            response
                .headers_mut()
                .insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
        }
        return response;
    }
    if request.uri().path() != "/mcp" {
        return StatusCode::NOT_FOUND.into_response();
    }
    if request.method() != Method::POST {
        let mut response = StatusCode::METHOD_NOT_ALLOWED.into_response();
        response
            .headers_mut()
            .insert(header::ALLOW, HeaderValue::from_static("POST"));
        return response;
    }
    match one_header(request.headers(), "content-type") {
        Ok(Some(value))
            if value
                .split(';')
                .next()
                .unwrap_or("")
                .trim()
                .eq_ignore_ascii_case("application/json") => {}
        _ => return StatusCode::UNSUPPORTED_MEDIA_TYPE.into_response(),
    }
    if !accepts_json(request.headers()) {
        return StatusCode::NOT_ACCEPTABLE.into_response();
    }
    let bytes = tokio::select! {
        biased;
        _ = stopped(state.shutdown.subscribe()) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
        result = tokio::time::timeout(BODY_TIMEOUT, to_bytes(request.into_body(), MAX_BODY)) => match result {
            Ok(Ok(bytes)) => bytes,
            Ok(Err(_)) => return StatusCode::PAYLOAD_TOO_LARGE.into_response(),
            Err(_) => return StatusCode::REQUEST_TIMEOUT.into_response(),
        },
    };
    let message: Value = match serde_json::from_slice(&bytes) {
        Ok(value) => value,
        Err(_) => return rpc_error(StatusCode::BAD_REQUEST, Value::Null, -32700, "Parse error"),
    };
    let Some(object) = message.as_object() else {
        return rpc_error(
            StatusCode::BAD_REQUEST,
            Value::Null,
            -32600,
            "Invalid Request",
        );
    };
    let id = object.get("id").cloned();
    if object.get("jsonrpc").and_then(Value::as_str) != Some("2.0")
        || id.as_ref().is_some_and(|value| !valid_id(value))
        || object.get("params").is_some_and(|value| !value.is_object())
    {
        return rpc_error(
            StatusCode::BAD_REQUEST,
            Value::Null,
            -32600,
            "Invalid Request",
        );
    }
    let Some(method) = object.get("method").and_then(Value::as_str) else {
        if !object.contains_key("method")
            && id.is_some()
            && (object.contains_key("result") ^ object.contains_key("error"))
        {
            return StatusCode::ACCEPTED.into_response();
        }
        return rpc_error(
            StatusCode::BAD_REQUEST,
            Value::Null,
            -32600,
            "Invalid Request",
        );
    };
    let params = object.get("params");
    let Some(id) = id else {
        match method {
            "notifications/initialized" => {
                let mut control = state
                    .control
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                if control.negotiated.is_none() {
                    return StatusCode::BAD_REQUEST.into_response();
                }
                control.initialized = true;
            }
            "notifications/cancelled" => {
                if let Some(id) = params
                    .and_then(|params| params.get("requestId"))
                    .filter(|id| valid_id(id))
                {
                    let mut control = state
                        .control
                        .lock()
                        .unwrap_or_else(|error| error.into_inner());
                    if let Some(cancel) = control
                        .calls
                        .get_mut(&id.to_string())
                        .and_then(Option::take)
                    {
                        let _ = cancel.send(());
                    }
                }
            }
            _ => {}
        }
        return StatusCode::ACCEPTED.into_response();
    };

    match method {
        "initialize" => {
            let version = params
                .and_then(|params| params.get("protocolVersion"))
                .and_then(Value::as_str);
            let capabilities = params.and_then(|params| params.get("capabilities"));
            let info = params.and_then(|params| params.get("clientInfo"));
            if version.is_none()
                || !capabilities.is_some_and(Value::is_object)
                || !info.is_some_and(|value| {
                    value.get("name").is_some_and(Value::is_string)
                        && value.get("version").is_some_and(Value::is_string)
                })
            {
                return rpc_error(StatusCode::OK, id, -32602, "Invalid initialize parameters");
            }
            let version = VERSIONS
                .iter()
                .copied()
                .find(|supported| Some(*supported) == version)
                .unwrap_or("2025-11-25");
            {
                let mut control = state
                    .control
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                control.negotiated = Some(version);
                control.initialized = false;
            }
            rpc_result(
                id,
                json!({
                    "protocolVersion": version,
                    "capabilities": {"tools": {"listChanged": false}},
                    "serverInfo": {"name": "kivio-voice", "version": env!("CARGO_PKG_VERSION")},
                }),
            )
        }
        "ping" => rpc_result(id, json!({})),
        "tools/list" | "tools/call" => {
            if !state
                .control
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .initialized
            {
                return rpc_error(StatusCode::OK, id, -32002, "Server not initialized");
            }
            if method == "tools/list" {
                if params.and_then(|params| params.get("cursor")).is_some() {
                    return rpc_error(StatusCode::OK, id, -32602, "Invalid cursor");
                }
                return rpc_result(id, json!({"tools": state.tools}));
            }
            let Some(name) = params
                .and_then(|params| params.get("name"))
                .and_then(Value::as_str)
            else {
                return rpc_error(StatusCode::OK, id, -32602, "Invalid tool name");
            };
            if !state
                .tools
                .iter()
                .any(|tool| tool.get("name").and_then(Value::as_str) == Some(name))
            {
                return rpc_error(StatusCode::OK, id, -32602, "Unknown tool");
            }
            let empty = json!({});
            let arguments = params
                .and_then(|params| params.get("arguments"))
                .unwrap_or(&empty);
            if !arguments.is_object() {
                return rpc_error(StatusCode::OK, id, -32602, "Invalid tool arguments");
            }
            let permit = match state.call_slots.clone().try_acquire_owned() {
                Ok(permit) => permit,
                Err(_) => {
                    return rpc_error(StatusCode::OK, id, -32000, "Too many concurrent tool calls")
                }
            };
            let (cancel, cancelled) = oneshot::channel();
            let key = id.to_string();
            {
                let mut control = state
                    .control
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                if control.calls.contains_key(&key) {
                    return rpc_error(StatusCode::OK, id, -32600, "Duplicate request ID");
                }
                control.calls.insert(key.clone(), Some(cancel));
                state.backend.working(control.calls.len());
            }
            let _call = PendingCall {
                state: state.clone(),
                key,
                _permit: permit,
            };
            let result = tokio::select! {
                biased;
                _ = stopped(state.shutdown.subscribe()) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
                _ = cancelled => return StatusCode::ACCEPTED.into_response(),
                result = tokio::time::timeout(TOOL_TIMEOUT, state.backend.call(name, arguments)) => match result {
                    Ok(result) => result,
                    Err(_) => Err("Voice tool timed out; its outcome may be unknown".to_string()),
                },
            };
            let (text, is_error) = match result {
                Ok(text) => (text, false),
                Err(error) => (error, true),
            };
            rpc_result(
                id,
                json!({"content": [{"type": "text", "text": text}], "isError": is_error}),
            )
        }
        _ => rpc_error(StatusCode::OK, id, -32601, "Method not found"),
    }
}

struct PendingCall<B: Backend> {
    state: Arc<ServerState<B>>,
    key: String,
    _permit: OwnedSemaphorePermit,
}

impl<B: Backend> Drop for PendingCall<B> {
    fn drop(&mut self) {
        let mut control = self
            .state
            .control
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if control.calls.remove(&self.key).is_some() {
            self.state.backend.working(control.calls.len());
        }
    }
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
        async fn call(&self, _name: &str, arguments: &Value) -> Result<String, String> {
            self.started.notify_one();
            self.release.notified().await;
            if arguments.get("fail") == Some(&json!(true)) {
                Err("tool failed".into())
            } else {
                Ok("tool result".into())
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
        server: &VoiceMcp,
        message: Value,
    ) -> reqwest::RequestBuilder {
        client
            .post(&server.url)
            .header(header::AUTHORIZATION, &server.authorization)
            .header(header::ACCEPT, "application/json, text/event-stream")
            .json(&message)
    }

    async fn initialize(client: &reqwest::Client, server: &VoiceMcp, requested: &str) -> Value {
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
