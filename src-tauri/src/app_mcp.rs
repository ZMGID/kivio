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
use serde::Deserialize;
use serde_json::{json, Value};
use tauri::AppHandle;
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    net::{TcpListener, TcpStream},
    sync::{oneshot, watch, OwnedSemaphorePermit, Semaphore},
    task::JoinHandle,
    time::{Instant, Sleep},
};

use crate::html_render;

pub(crate) const CHAT_MCP_SERVER_NAME: &str = "kivio";
pub(crate) const MAX_BODY: usize = 256 * 1024;
pub(crate) const MAX_CALLS: usize = 8;
const MAX_CHAT_BODY: usize = 4 * 1024 * 1024;
const CHAT_TIMEOUT_MESSAGE: &str = "Chat tool timed out; its outcome may be unknown";
const MAX_CONNECTIONS: usize = 32;
const HEADER_TIMEOUT: Duration = Duration::from_secs(10);
const BODY_TIMEOUT: Duration = Duration::from_secs(10);
// The voice orchestrator can itself wait ten minutes for a delegated reply.
const TOOL_TIMEOUT: Duration = Duration::from_secs(620);
const CONNECTION_TIMEOUT: Duration = Duration::from_secs(650);
const VERSIONS: &[&str] = &["2025-06-18", "2025-11-25"];

pub(crate) struct ToolOutput {
    pub text: String,
    pub structured: Option<Value>,
    pub image_png_base64: Option<String>,
}

impl From<String> for ToolOutput {
    fn from(text: String) -> Self {
        Self {
            text,
            structured: None,
            image_png_base64: None,
        }
    }
}
impl From<html_render::HtmlToolResult> for ToolOutput {
    fn from(result: html_render::HtmlToolResult) -> Self {
        Self {
            text: result.text,
            structured: result.structured,
            image_png_base64: result.image_png_base64,
        }
    }
}

pub(crate) trait Backend: Clone + Send + Sync + 'static {
    fn call(
        &self,
        name: &str,
        arguments: &Value,
    ) -> impl Future<Output = Result<ToolOutput, String>> + Send;

    fn complete(&self, _name: &str, output: ToolOutput) -> Result<ToolOutput, String> {
        Ok(output)
    }

    fn revoke(&self) {}

    fn working(&self, _count: usize) {}
}

struct Control {
    negotiated: Option<&'static str>,
    initialized: bool,
    calls: HashMap<String, Option<oneshot::Sender<()>>>,
}

struct Session<B> {
    backend: B,
    control: Mutex<Control>,
    shutdown: watch::Sender<bool>,
}

struct ServerState<B> {
    host: String,
    origin: String,
    name: &'static str,
    label: &'static str,
    timeout_message: &'static str,
    body_limit: usize,
    tools: Vec<Value>,
    sessions: Mutex<HashMap<String, Arc<Session<B>>>>,
    call_slots: Arc<Semaphore>,
    shutdown: watch::Sender<bool>,
}

struct ServerOwner<B> {
    state: Arc<ServerState<B>>,
    task: JoinHandle<()>,
}

impl<B> Drop for ServerOwner<B> {
    fn drop(&mut self) {
        self.state.shutdown.send_replace(true);
        self.task.abort();
    }
}

pub(crate) struct Server<B> {
    owner: Arc<ServerOwner<B>>,
}

pub(crate) struct McpCredential {
    pub url: String,
    pub authorization: String,
    _lease: CredentialLease<ChatBackend>,
}

pub(crate) struct ServerCredential<B: Backend> {
    pub url: String,
    pub authorization: String,
    _lease: CredentialLease<B>,
}

struct CredentialLease<B: Backend> {
    server: Arc<ServerOwner<B>>,
    session: Arc<Session<B>>,
}

impl From<ServerCredential<ChatBackend>> for McpCredential {
    fn from(credential: ServerCredential<ChatBackend>) -> Self {
        Self {
            url: credential.url,
            authorization: credential.authorization,
            _lease: credential._lease,
        }
    }
}

impl<B: Backend> Drop for CredentialLease<B> {
    fn drop(&mut self) {
        // The public authorization can be changed; revoke the originally issued session.
        self.server
            .state
            .sessions
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .retain(|_, session| !Arc::ptr_eq(session, &self.session));
        // Completion records and revocation purge share this lock.
        let _control = self
            .session
            .control
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        self.session.shutdown.send_replace(true);
        self.session.backend.revoke();
    }
}

impl<B: Backend> Server<B> {
    pub(crate) fn issue(&self, backend: B) -> Result<ServerCredential<B>, String> {
        let mut sessions = self
            .owner
            .state
            .sessions
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        loop {
            let mut secret = [0u8; 32];
            rand::rngs::OsRng.try_fill_bytes(&mut secret).map_err(|_| {
                format!(
                    "Failed to generate {} MCP credential",
                    self.owner.state.label
                )
            })?;
            let authorization = format!("Bearer {}", URL_SAFE_NO_PAD.encode(secret));
            if sessions.contains_key(&authorization) {
                continue;
            }
            let (shutdown, _) = watch::channel(false);
            let session = Arc::new(Session {
                backend,
                control: Mutex::new(Control {
                    negotiated: None,
                    initialized: false,
                    calls: HashMap::new(),
                }),
                shutdown,
            });
            sessions.insert(authorization.clone(), session.clone());
            return Ok(ServerCredential {
                url: format!("{}/mcp", self.owner.state.origin),
                authorization,
                _lease: CredentialLease {
                    server: self.owner.clone(),
                    session,
                },
            });
        }
    }
}

pub(crate) async fn start_server<B: Backend>(
    tools: Vec<Value>,
    name: &'static str,
    body_limit: usize,
    label: &'static str,
    timeout_message: &'static str,
) -> Result<Server<B>, String> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|error| format!("Failed to bind {label} MCP listener: {error}"))?;
    let address = listener
        .local_addr()
        .map_err(|error| format!("Failed to inspect {label} MCP listener: {error}"))?;
    let host = address.to_string();
    let origin = format!("http://{host}");
    let (shutdown, _) = watch::channel(false);
    let state = Arc::new(ServerState {
        host,
        origin,
        name,
        label,
        timeout_message,
        body_limit,
        tools,
        sessions: Mutex::new(HashMap::new()),
        call_slots: Arc::new(Semaphore::new(MAX_CALLS)),
        shutdown: shutdown.clone(),
    });
    let listener = LimitedListener {
        listener,
        slots: Arc::new(Semaphore::new(MAX_CONNECTIONS)),
        shutdown: shutdown.subscribe(),
    };
    let router = Router::new()
        .fallback(endpoint::<B>)
        .with_state(state.clone());
    let task = tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    Ok(Server {
        owner: Arc::new(ServerOwner { state, task }),
    })
}

#[derive(Clone)]
pub(crate) enum ChatBackend {
    App {
        app: AppHandle,
        conversation_id: String,
    },
    #[cfg(test)]
    Test,
    #[cfg(test)]
    TestRender {
        attachments_dir: std::path::PathBuf,
        conversation_id: String,
    },
}
impl ChatBackend {
    fn conversation_id(&self) -> Option<&str> {
        match self {
            Self::App {
                conversation_id, ..
            } => Some(conversation_id),
            #[cfg(test)]
            Self::TestRender {
                conversation_id, ..
            } => Some(conversation_id),
            #[cfg(test)]
            Self::Test => None,
        }
    }
}

impl Backend for ChatBackend {
    async fn call(&self, name: &str, arguments: &Value) -> Result<ToolOutput, String> {
        match self {
            Self::App {
                app,
                conversation_id,
            } => html_render::call_tool(app, conversation_id, name, arguments)
                .await
                .map(Into::into),
            #[cfg(test)]
            Self::Test => Err("Test credential has no tool backend".to_string()),
            #[cfg(test)]
            Self::TestRender {
                attachments_dir, ..
            } => {
                if name != html_render::HTML_RENDER_TOOL {
                    return Err(format!("Unknown HTML tool: {name}"));
                }
                html_render::call_render_in_directory(attachments_dir, arguments)
                    .await
                    .map(Into::into)
            }
        }
    }

    fn complete(&self, name: &str, output: ToolOutput) -> Result<ToolOutput, String> {
        match self.conversation_id() {
            Some(conversation_id) => chat_tool_output(conversation_id, name, output),
            None => Ok(output),
        }
    }

    fn revoke(&self) {
        if let Some(conversation_id) = self.conversation_id() {
            html_render::clear_pending_renders(conversation_id);
        }
    }
}

fn chat_tool_output(
    conversation_id: &str,
    name: &str,
    result: ToolOutput,
) -> Result<ToolOutput, String> {
    if name == html_render::HTML_RENDER_TOOL {
        let structured = result
            .structured
            .as_ref()
            .ok_or_else(|| "HTML render returned no reference".to_string())?;
        let reference = html_render::HtmlRenderReference::deserialize(structured)
            .map_err(|error| format!("Invalid HTML render reference: {error}"))?;
        html_render::record_pending_render(conversation_id, reference);
    }
    Ok(result)
}

fn complete_tool_call<B: Backend>(
    session: &Session<B>,
    name: &str,
    output: ToolOutput,
) -> Option<Result<ToolOutput, String>> {
    let _control = session
        .control
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if *session.shutdown.borrow() {
        return None;
    }
    Some(session.backend.complete(name, output))
}

static CHAT_SERVER: tokio::sync::OnceCell<Server<ChatBackend>> = tokio::sync::OnceCell::const_new();

pub(crate) async fn issue_chat_credential(
    app: &AppHandle,
    conversation_id: &str,
) -> Result<McpCredential, String> {
    let server = CHAT_SERVER
        .get_or_try_init(|| async {
            start_server(
                html_render::tool_definitions(),
                CHAT_MCP_SERVER_NAME,
                MAX_CHAT_BODY,
                "chat",
                CHAT_TIMEOUT_MESSAGE,
            )
            .await
        })
        .await?;
    server
        .issue(ChatBackend::App {
            app: app.clone(),
            conversation_id: conversation_id.to_string(),
        })
        .map(McpCredential::from)
}

#[cfg(test)]
pub(crate) struct TestCredentialProbe {
    server: Arc<ServerOwner<ChatBackend>>,
    authorization: String,
}

#[cfg(test)]
impl TestCredentialProbe {
    pub(crate) fn is_active(&self) -> bool {
        self.server
            .state
            .sessions
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .contains_key(&self.authorization)
    }
}

#[cfg(test)]
pub(crate) async fn issue_test_chat_credential(
) -> Result<(McpCredential, TestCredentialProbe), String> {
    let server = start_server(
        html_render::tool_definitions(),
        CHAT_MCP_SERVER_NAME,
        MAX_CHAT_BODY,
        "chat",
        CHAT_TIMEOUT_MESSAGE,
    )
    .await?;
    let credential: McpCredential = server.issue(ChatBackend::Test)?.into();
    let probe = TestCredentialProbe {
        server: server.owner.clone(),
        authorization: credential.authorization.clone(),
    };
    Ok((credential, probe))
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

pub(crate) fn same_secret(actual: &str, expected: &str) -> bool {
    actual.len() == expected.len()
        && actual
            .as_bytes()
            .iter()
            .zip(expected.as_bytes())
            .fold(0u8, |diff, (a, b)| diff | (a ^ b))
            == 0
}

fn authorize<B>(
    state: &ServerState<B>,
    request: &Request<Body>,
) -> Result<Arc<Session<B>>, StatusCode> {
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
    let authorization = one_header(headers, "authorization")
        .ok()
        .flatten()
        .ok_or(StatusCode::UNAUTHORIZED)?;
    let session = state
        .sessions
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .iter()
        .find_map(|(secret, session)| same_secret(authorization, secret).then(|| session.clone()))
        .ok_or(StatusCode::UNAUTHORIZED)?;
    match one_header(headers, "mcp-protocol-version") {
        Ok(None) => {}
        Ok(Some(version)) if VERSIONS.contains(&version) => {}
        _ => return Err(StatusCode::BAD_REQUEST),
    }
    Ok(session)
}

pub(crate) fn accepts_json(headers: &HeaderMap) -> bool {
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
    let session = match authorize(&state, &request) {
        Ok(session) => session,
        Err(status) => {
            let mut response = status.into_response();
            if status == StatusCode::UNAUTHORIZED {
                response
                    .headers_mut()
                    .insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
            }
            return response;
        }
    };
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
        _ = stopped(session.shutdown.subscribe()) => return StatusCode::UNAUTHORIZED.into_response(),
        result = tokio::time::timeout(BODY_TIMEOUT, to_bytes(request.into_body(), state.body_limit)) => match result {
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
                let mut control = session
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
                    let mut control = session
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
                let mut control = session
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
                    "serverInfo": {"name": state.name, "version": env!("CARGO_PKG_VERSION")},
                }),
            )
        }
        "ping" => rpc_result(id, json!({})),
        "tools/list" | "tools/call" => {
            if !session
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
                let mut control = session
                    .control
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                if control.calls.contains_key(&key) {
                    return rpc_error(StatusCode::OK, id, -32600, "Duplicate request ID");
                }
                control.calls.insert(key.clone(), Some(cancel));
                session.backend.working(control.calls.len());
            }
            let _call = PendingCall {
                session: session.clone(),
                key,
                _permit: permit,
            };
            let result = tokio::select! {
                biased;
                _ = stopped(state.shutdown.subscribe()) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
                _ = stopped(session.shutdown.subscribe()) => return StatusCode::UNAUTHORIZED.into_response(),
                _ = cancelled => return StatusCode::ACCEPTED.into_response(),
                result = tokio::time::timeout(TOOL_TIMEOUT, session.backend.call(name, arguments)) => match result {
                    Ok(result) => result,
                    Err(_) => Err(state.timeout_message.to_string()),
                },
            };
            let result = match result {
                Ok(output) => match complete_tool_call(&session, name, output) {
                    Some(result) => result,
                    None => return StatusCode::UNAUTHORIZED.into_response(),
                },
                Err(error) => Err(error),
            };
            let (output, is_error) = match result {
                Ok(output) => (output, false),
                Err(error) => (ToolOutput::from(error), true),
            };
            let mut content = vec![json!({"type": "text", "text": output.text})];
            if let Some(image) = output.image_png_base64 {
                content.push(json!({"type": "image", "data": image, "mimeType": "image/png"}));
            }
            let mut result = json!({"content": content, "isError": is_error});
            if let Some(structured) = output.structured {
                result["structuredContent"] = structured;
            }
            rpc_result(id, result)
        }
        _ => rpc_error(StatusCode::OK, id, -32601, "Method not found"),
    }
}

struct PendingCall<B: Backend> {
    session: Arc<Session<B>>,
    key: String,
    _permit: OwnedSemaphorePermit,
}

impl<B: Backend> Drop for PendingCall<B> {
    fn drop(&mut self) {
        let mut control = self
            .session
            .control
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if control.calls.remove(&self.key).is_some() {
            self.session.backend.working(control.calls.len());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::Notify;

    #[derive(Clone)]
    struct ScopedBackend {
        conversation: &'static str,
        started: Arc<Notify>,
    }

    impl Backend for ScopedBackend {
        async fn call(&self, name: &str, arguments: &Value) -> Result<ToolOutput, String> {
            if arguments.get("wait") == Some(&json!(true)) {
                self.started.notify_one();
                std::future::pending::<()>().await;
            }
            Ok(ToolOutput {
                text: if name == html_render::HTML_RENDER_TOOL {
                    "Rendered page htmlrender-00000000-0000-4000-8000-000000000000.".to_string()
                } else {
                    self.conversation.to_string()
                },
                structured: Some(if name == html_render::HTML_RENDER_TOOL {
                    json!({
                        "type": "html_render",
                        "attachmentId": "htmlrender-00000000-0000-4000-8000-000000000000",
                        "title": self.conversation,
                        "height": 480
                    })
                } else {
                    json!({"width": 728})
                }),
                image_png_base64: (name == html_render::HTML_PREVIEW_TOOL)
                    .then(|| "aGVsbG8=".to_string()),
            })
        }
    }

    fn backend(conversation: &'static str) -> ScopedBackend {
        ScopedBackend {
            conversation,
            started: Arc::new(Notify::new()),
        }
    }

    fn client() -> reqwest::Client {
        reqwest::Client::builder().no_proxy().build().unwrap()
    }

    fn post<B: Backend>(
        client: &reqwest::Client,
        credential: &ServerCredential<B>,
        message: Value,
    ) -> reqwest::RequestBuilder {
        client
            .post(&credential.url)
            .header(header::AUTHORIZATION, &credential.authorization)
            .json(&message)
    }

    async fn initialize<B: Backend>(client: &reqwest::Client, credential: &ServerCredential<B>) {
        let response = post(
            client,
            credential,
            json!({
                "jsonrpc": "2.0", "id": 1, "method": "initialize",
                "params": {"protocolVersion": "2025-11-25", "capabilities": {},
                    "clientInfo": {"name": "test", "version": "1"}}
            }),
        )
        .send()
        .await
        .unwrap();
        let body: Value = response.json().await.unwrap();
        assert_eq!(body["result"]["serverInfo"]["name"], CHAT_MCP_SERVER_NAME);
        let response = post(
            client,
            credential,
            json!({
                "jsonrpc": "2.0", "method": "notifications/initialized"
            }),
        )
        .send()
        .await
        .unwrap();
        assert_eq!(response.status(), StatusCode::ACCEPTED);
    }

    async fn server() -> Server<ScopedBackend> {
        start_server(
            html_render::tool_definitions(),
            CHAT_MCP_SERVER_NAME,
            MAX_CHAT_BODY,
            "chat",
            CHAT_TIMEOUT_MESSAGE,
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn chat_http_publishes_real_html_only_for_the_authorized_conversation() {
        let temp = tempfile::tempdir().unwrap();
        let first_dir = temp.path().join("first");
        let second_dir = temp.path().join("second");
        std::fs::create_dir_all(&first_dir).unwrap();
        std::fs::create_dir_all(&second_dir).unwrap();
        let first_conversation = format!("mcp-http-first-{}", uuid::Uuid::new_v4());
        let second_conversation = format!("mcp-http-second-{}", uuid::Uuid::new_v4());
        let server = start_server::<ChatBackend>(
            html_render::tool_definitions(),
            CHAT_MCP_SERVER_NAME,
            MAX_CHAT_BODY,
            "chat",
            CHAT_TIMEOUT_MESSAGE,
        )
        .await
        .unwrap();
        let first = server
            .issue(ChatBackend::TestRender {
                attachments_dir: first_dir.clone(),
                conversation_id: first_conversation.clone(),
            })
            .unwrap();
        let second = server
            .issue(ChatBackend::TestRender {
                attachments_dir: second_dir.clone(),
                conversation_id: second_conversation.clone(),
            })
            .unwrap();
        let other_server = start_server::<ChatBackend>(
            html_render::tool_definitions(),
            CHAT_MCP_SERVER_NAME,
            MAX_CHAT_BODY,
            "chat",
            CHAT_TIMEOUT_MESSAGE,
        )
        .await
        .unwrap();
        let other_token = other_server.issue(ChatBackend::Test).unwrap();
        let client = client();
        initialize(&client, &first).await;
        initialize(&client, &second).await;

        let listed: Value = post(
            &client,
            &first,
            json!({
                "jsonrpc": "2.0", "id": 2, "method": "tools/list"
            }),
        )
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
        assert_eq!(
            listed["result"]["tools"],
            json!(html_render::tool_definitions())
        );

        let title = "HTTP render";
        let first_html = "<html><body><p>first conversation</p></body></html>";
        let second_html = "<html><body><p>second conversation</p></body></html>";
        let first_call = json!({
            "jsonrpc": "2.0", "id": 3, "method": "tools/call",
            "params": {"name": html_render::HTML_RENDER_TOOL, "arguments": {
                "html": first_html, "title": title, "height": 480,
                "conversationId": &second_conversation
            }}
        });
        let denied = client
            .post(&first.url)
            .header(header::AUTHORIZATION, &other_token.authorization)
            .json(&first_call)
            .send()
            .await
            .unwrap();
        assert_eq!(denied.status(), StatusCode::UNAUTHORIZED);
        assert!(std::fs::read_dir(&first_dir).unwrap().next().is_none());

        let invalid: Value = post(
            &client,
            &first,
            json!({
                "jsonrpc": "2.0", "id": 4, "method": "tools/call",
                "params": {"name": html_render::HTML_RENDER_TOOL, "arguments": {
                    "html": first_html, "title": title, "height": 2001
                }}
            }),
        )
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
        assert_eq!(invalid["result"]["isError"], true);
        assert!(std::fs::read_dir(&first_dir).unwrap().next().is_none());

        let first_result: Value = post(&client, &first, first_call)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(first_result["result"]["isError"], false);
        assert_eq!(first_result["result"]["content"][0]["type"], "text");
        let first_reference = first_result["result"]["structuredContent"].clone();
        assert_eq!(first_reference["type"], "html_render");
        assert_eq!(first_reference["title"], title);
        assert_eq!(first_reference["height"], 480);
        let first_id = first_reference["attachmentId"].as_str().unwrap();
        assert!(first_result["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .starts_with(&format!("Rendered page {first_id}.")));
        assert_eq!(
            std::fs::read_to_string(first_dir.join(format!("{first_id}.html"))).unwrap(),
            first_html
        );
        assert!(html_render::take_pending_render(&second_conversation, first_id).is_none());

        let current_result: Value = post(
            &client,
            &first,
            json!({
                "jsonrpc": "2.0", "id": 6, "method": "tools/call",
                "params": {"name": html_render::HTML_RENDER_TOOL, "arguments": {
                    "html": "<p>new page with the same title</p>", "title": title, "height": 480
                }}
            }),
        )
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
        assert_eq!(current_result["result"]["isError"], false);
        let current_reference = current_result["result"]["structuredContent"].clone();
        let current_id = current_reference["attachmentId"].as_str().unwrap();
        assert_ne!(first_id, current_id);
        assert!(current_result["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .starts_with(&format!("Rendered page {current_id}.")));
        assert_eq!(
            serde_json::to_value(
                html_render::take_pending_render(&first_conversation, current_id).unwrap()
            )
            .unwrap(),
            current_reference
        );
        assert_eq!(
            serde_json::to_value(
                html_render::take_pending_render(&first_conversation, first_id).unwrap()
            )
            .unwrap(),
            first_reference
        );

        let abandoned_result: Value = post(
            &client,
            &first,
            json!({
                "jsonrpc": "2.0", "id": 7, "method": "tools/call",
                "params": {"name": html_render::HTML_RENDER_TOOL, "arguments": {
                    "html": "<p>abandoned on revoke</p>", "title": title, "height": 480
                }}
            }),
        )
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
        assert_eq!(abandoned_result["result"]["isError"], false);
        let abandoned_id = abandoned_result["result"]["structuredContent"]["attachmentId"]
            .as_str()
            .unwrap();
        assert!(abandoned_result["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .starts_with(&format!("Rendered page {abandoned_id}.")));

        let second_result: Value = post(
            &client,
            &second,
            json!({
                "jsonrpc": "2.0", "id": 5, "method": "tools/call",
                "params": {"name": html_render::HTML_RENDER_TOOL, "arguments": {
                    "html": second_html, "title": title, "height": 480,
                    "conversationId": &first_conversation
                }}
            }),
        )
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
        assert_eq!(second_result["result"]["isError"], false);
        let second_reference = second_result["result"]["structuredContent"].clone();
        let second_id = second_reference["attachmentId"].as_str().unwrap();
        assert!(second_result["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .starts_with(&format!("Rendered page {second_id}.")));
        assert_ne!(first_id, second_id);
        assert_eq!(
            std::fs::read_to_string(second_dir.join(format!("{second_id}.html"))).unwrap(),
            second_html
        );
        assert!(!first_dir.join(format!("{second_id}.html")).exists());
        assert!(!second_dir.join(format!("{first_id}.html")).exists());
        assert!(html_render::take_pending_render(&first_conversation, second_id).is_none());
        drop(first);
        assert!(html_render::take_pending_render(&first_conversation, abandoned_id).is_none());
        assert_eq!(
            serde_json::to_value(
                html_render::take_pending_render(&second_conversation, second_id).unwrap()
            )
            .unwrap(),
            second_reference
        );
    }

    #[tokio::test]
    async fn chat_http_scopes_credentials_and_preserves_tool_results() {
        let server = server().await;
        let first = server.issue(backend("first")).unwrap();
        let second = server.issue(backend("second")).unwrap();
        let client = client();
        assert_eq!(first.url, second.url);
        assert!(first.url.starts_with("http://127.0.0.1:"));
        assert!(!same_secret(&first.authorization, &second.authorization));

        initialize(&client, &first).await;
        let before_init: Value = post(
            &client,
            &second,
            json!({
                "jsonrpc": "2.0", "id": 2, "method": "tools/list"
            }),
        )
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
        assert_eq!(before_init["error"]["code"], -32002);
        initialize(&client, &second).await;

        let listed: Value = post(
            &client,
            &first,
            json!({
                "jsonrpc": "2.0", "id": 3, "method": "tools/list"
            }),
        )
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
        assert_eq!(
            listed["result"]["tools"],
            json!(html_render::tool_definitions())
        );

        for (credential, conversation) in [(&first, "first"), (&second, "second")] {
            let render: Value = post(
                &client,
                credential,
                json!({
                    "jsonrpc": "2.0", "id": 4, "method": "tools/call",
                    "params": {"name": html_render::HTML_RENDER_TOOL, "arguments": {}}
                }),
            )
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
            let id = render["result"]["structuredContent"]["attachmentId"]
                .as_str()
                .unwrap();
            assert!(render["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .starts_with(&format!("Rendered page {id}.")));
            assert_eq!(render["result"]["structuredContent"]["title"], conversation);
            assert_eq!(render["result"]["structuredContent"]["type"], "html_render");
            assert_eq!(render["result"]["isError"], false);
        }
        let preview: Value = post(
            &client,
            &first,
            json!({
                "jsonrpc": "2.0", "id": 5, "method": "tools/call",
                "params": {"name": html_render::HTML_PREVIEW_TOOL, "arguments": {}}
            }),
        )
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
        assert_eq!(
            preview["result"]["content"][1],
            json!({
                "type": "image", "data": "aGVsbG8=", "mimeType": "image/png"
            })
        );
        assert_eq!(preview["result"]["structuredContent"]["width"], 728);

        let ping = json!({"jsonrpc": "2.0", "id": 6, "method": "ping"});
        assert_eq!(
            client
                .post(&first.url)
                .json(&ping)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            post(&client, &first, ping.clone())
                .header(header::AUTHORIZATION, &first.authorization)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            post(&client, &first, ping.clone())
                .header(header::ORIGIN, "https://evil.example")
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            post(&client, &first, ping)
                .header(header::HOST, "evil.example")
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::FORBIDDEN
        );
    }

    #[tokio::test]
    async fn chat_revocation_cancels_only_its_own_calls() {
        let server = server().await;
        let first_backend = backend("first");
        let second_backend = backend("second");
        let first = server.issue(first_backend.clone()).unwrap();
        let second = server.issue(second_backend.clone()).unwrap();
        let client = client();
        initialize(&client, &first).await;
        initialize(&client, &second).await;

        let call = json!({"jsonrpc": "2.0", "id": 42, "method": "tools/call",
            "params": {"name": html_render::HTML_PREVIEW_TOOL, "arguments": {"wait": true}}});
        let first_request = post(&client, &first, call.clone());
        let first_task = tokio::spawn(async move { first_request.send().await });
        tokio::time::timeout(Duration::from_secs(3), first_backend.started.notified())
            .await
            .unwrap();
        let second_request = post(&client, &second, call);
        let second_task = tokio::spawn(async move { second_request.send().await });
        tokio::time::timeout(Duration::from_secs(3), second_backend.started.notified())
            .await
            .unwrap();

        let cancelled = post(
            &client,
            &second,
            json!({
                "jsonrpc": "2.0", "method": "notifications/cancelled", "params": {"requestId": 42}
            }),
        )
        .send()
        .await
        .unwrap();
        assert_eq!(cancelled.status(), StatusCode::ACCEPTED);
        assert_eq!(
            second_task.await.unwrap().unwrap().status(),
            StatusCode::ACCEPTED
        );
        assert!(!first_task.is_finished());

        let old_url = first.url.clone();
        let old_authorization = first.authorization.clone();
        drop(first);
        assert_eq!(
            first_task.await.unwrap().unwrap().status(),
            StatusCode::UNAUTHORIZED
        );
        let ping = json!({"jsonrpc": "2.0", "id": 9, "method": "ping"});
        assert_eq!(
            client
                .post(old_url)
                .header(header::AUTHORIZATION, old_authorization)
                .json(&ping)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            post(&client, &second, ping).send().await.unwrap().status(),
            StatusCode::OK
        );
    }

    #[tokio::test]
    async fn chat_accepts_full_escaped_html_but_still_bounds_bodies() {
        let server = server().await;
        let credential = server.issue(backend("first")).unwrap();
        let client = client();
        initialize(&client, &credential).await;
        assert!(TOOL_TIMEOUT >= Duration::from_secs(120));
        let accepted = post(&client, &credential, json!({
            "jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": {"name": html_render::HTML_RENDER_TOOL, "arguments": {"html": "\0".repeat(512_000)}}
        })).send().await.unwrap();
        assert_eq!(accepted.status(), StatusCode::OK);
        assert_eq!(
            accepted.json::<Value>().await.unwrap()["result"]["isError"],
            false
        );
        let rejected = client
            .post(&credential.url)
            .header(header::AUTHORIZATION, &credential.authorization)
            .header(header::CONTENT_TYPE, "application/json")
            .body(" ".repeat(MAX_CHAT_BODY + 1))
            .send()
            .await
            .unwrap();
        assert_eq!(rejected.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[test]
    fn only_successful_mcp_renders_enter_the_conversation_ledger() {
        let conversation = "mcp-ledger-test";
        let title = "mcp-ledger-title";
        let id = "htmlrender-00000000-0000-4000-8000-000000000000";
        let reference = json!({
            "type": "html_render",
            "attachmentId": id,
            "title": title,
            "height": 480
        });
        let render = ToolOutput {
            text: format!("Rendered page {id}."),
            structured: Some(reference.clone()),
            image_png_base64: None,
        };
        let output = chat_tool_output(conversation, html_render::HTML_RENDER_TOOL, render).unwrap();
        assert_eq!(output.structured, Some(reference.clone()));
        assert!(output.text.starts_with(&format!("Rendered page {id}.")));
        assert!(html_render::take_pending_render("another-conversation", id).is_none());
        let recorded = html_render::take_pending_render(conversation, id).unwrap();
        assert_eq!(serde_json::to_value(recorded).unwrap(), reference);
        assert!(html_render::take_pending_render(conversation, id).is_none());

        let preview = ToolOutput {
            text: "preview".to_string(),
            structured: Some(reference),
            image_png_base64: Some("aGVsbG8=".to_string()),
        };
        assert!(chat_tool_output(conversation, html_render::HTML_PREVIEW_TOOL, preview).is_ok());
        assert!(html_render::take_pending_render(conversation, id).is_none());
        let missing = ToolOutput {
            text: "bad render".to_string(),
            structured: None,
            image_png_base64: None,
        };
        assert!(chat_tool_output(conversation, html_render::HTML_RENDER_TOOL, missing).is_err());
        assert!(html_render::take_pending_render(conversation, id).is_none());
    }

    #[tokio::test]
    async fn revoked_credential_cannot_record_a_late_render() {
        let temp = tempfile::tempdir().unwrap();
        let conversation = format!("mcp-late-{}", uuid::Uuid::new_v4());
        let server = start_server::<ChatBackend>(
            html_render::tool_definitions(),
            CHAT_MCP_SERVER_NAME,
            MAX_CHAT_BODY,
            "chat",
            CHAT_TIMEOUT_MESSAGE,
        )
        .await
        .unwrap();
        let old: McpCredential = server
            .issue(ChatBackend::TestRender {
                attachments_dir: temp.path().to_path_buf(),
                conversation_id: conversation.clone(),
            })
            .unwrap()
            .into();
        let old_session = old._lease.session.clone();
        let (ready, published) = oneshot::channel();
        let (resume, release) = oneshot::channel();
        let late = tokio::spawn(async move {
            let arguments =
                json!({"html": "<p>old page</p>", "title": "same title", "height": 480});
            let output = old_session
                .backend
                .call(html_render::HTML_RENDER_TOOL, &arguments)
                .await
                .unwrap();
            let id = output.structured.as_ref().unwrap()["attachmentId"]
                .as_str()
                .unwrap()
                .to_string();
            ready.send(id).unwrap();
            release.await.unwrap();
            complete_tool_call(&old_session, html_render::HTML_RENDER_TOOL, output)
        });
        let old_id = tokio::time::timeout(Duration::from_secs(3), published)
            .await
            .unwrap()
            .unwrap();
        drop(old);

        let current = server
            .issue(ChatBackend::TestRender {
                attachments_dir: temp.path().to_path_buf(),
                conversation_id: conversation.clone(),
            })
            .unwrap();
        let arguments = json!({"html": "<p>new page</p>", "title": "same title", "height": 480});
        let output = current
            ._lease
            .session
            .backend
            .call(html_render::HTML_RENDER_TOOL, &arguments)
            .await
            .unwrap();
        let reference = output.structured.as_ref().unwrap().clone();
        let current_id = reference["attachmentId"].as_str().unwrap();
        assert_ne!(old_id, current_id);
        assert!(output
            .text
            .starts_with(&format!("Rendered page {current_id}.")));
        complete_tool_call(
            &current._lease.session,
            html_render::HTML_RENDER_TOOL,
            output,
        )
        .unwrap()
        .unwrap();

        resume.send(()).unwrap();
        assert!(late.await.unwrap().is_none());
        assert!(html_render::take_pending_render(&conversation, &old_id).is_none());
        assert_eq!(
            serde_json::to_value(
                html_render::take_pending_render(&conversation, current_id).unwrap()
            )
            .unwrap(),
            reference
        );
    }

    #[tokio::test]
    async fn test_credential_probe_tracks_real_revocation() {
        let (mut credential, probe) = issue_test_chat_credential().await.unwrap();
        assert!(probe.is_active());
        credential.authorization.clear();
        drop(credential);
        assert!(!probe.is_active());
    }
}
