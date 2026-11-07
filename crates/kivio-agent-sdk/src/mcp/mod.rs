//! Explicit MCP clients and authorized adapters for the tool registry.
//!
//! A client owns one transport. Hosts supply credentials and decide when to create
//! a replacement. Call [`McpClient::close`] to await request and process cleanup.

use std::{
    collections::{HashMap, HashSet, VecDeque},
    fmt,
    future::Future,
    path::PathBuf,
    sync::{Arc, Mutex as StdMutex},
    time::Duration,
};

use rmcp::{
    model::{
        CallToolRequest, CallToolRequestParams, ClientRequest, ListToolsRequest,
        PaginatedRequestParams, PingRequest, ServerResult,
    },
    service::{Peer, PeerRequestOptions, RoleClient},
};
use serde_json::Value;
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    sync::{Mutex, Notify},
};

mod conn;
mod registry;
mod result;
mod types;
pub use conn::{
    is_method_not_found, is_oauth_error, HANDSHAKE_TIMEOUT, LIST_TOOLS_TIMEOUT, PING_TIMEOUT,
};
pub use registry::{
    compact_mcp_tool_id, mcp_tool_definition, sanitize_openai_tool_name, wire_mcp_tool_id,
};
pub use result::parse_tool_result;
pub use types::{McpTool, McpToolCallResult};

const CLEANUP_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone)]
pub enum McpTransport {
    Stdio {
        command: String,
        args: Vec<String>,
        cwd: Option<PathBuf>,
        env: HashMap<String, String>,
    },
    StreamableHttp {
        url: String,
        headers: HashMap<String, String>,
    },
}

impl fmt::Debug for McpTransport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Stdio { .. } => "Stdio { .. }",
            Self::StreamableHttp { .. } => "StreamableHttp { .. }",
        })
    }
}

#[derive(Clone, Debug)]
pub struct McpClientConfig {
    pub transport: McpTransport,
}

#[derive(Debug)]
pub struct McpCallFailure {
    pub message: String,
    /// Only a rejected local send is safe for the host to replay.
    pub replay_safe: bool,
}

#[derive(Default)]
struct Lifecycle {
    closed: bool,
    in_flight: usize,
}

struct ToolCache {
    revision: u64,
    tools: Vec<McpTool>,
}

pub struct McpClient {
    service: Mutex<Option<conn::McpService>>,
    peer: Peer<RoleClient>,
    lifecycle: StdMutex<Lifecycle>,
    closing: Notify,
    drained: Notify,
    revision: Arc<std::sync::atomic::AtomicU64>,
    cache: StdMutex<Option<ToolCache>>,
    refresh: Mutex<()>,
    child_pid: Option<u32>,
    stderr: Arc<Mutex<VecDeque<String>>>,
    stderr_task: StdMutex<Option<tokio::task::JoinHandle<()>>>,
    secrets: Vec<String>,
}

impl fmt::Debug for McpClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("McpClient")
            .field("closed", &self.is_closed())
            .finish_non_exhaustive()
    }
}

impl McpClient {
    /// Connect, negotiate, and discover the initial tool catalog.
    pub async fn connect(config: McpClientConfig, http: reqwest::Client) -> Result<Self, String> {
        let secrets = configured_secrets(&config);
        let connected = conn::connect(&config, &http)
            .await
            .map_err(|e| redact(e, &secrets))?;
        let stderr = Arc::new(Mutex::new(VecDeque::new()));
        let stderr_task = connected.stderr.map(|pipe| {
            let tail = stderr.clone();
            let secrets = secrets.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(pipe).lines();
                loop {
                    let line = match lines.next_line().await {
                        Ok(Some(line)) => line,
                        Ok(None) => break,
                        Err(e) if e.kind() == std::io::ErrorKind::InvalidData => continue,
                        Err(_) => break,
                    };
                    let mut tail = tail.lock().await;
                    if tail.len() == 20 {
                        tail.pop_front();
                    }
                    tail.push_back(redact(line, &secrets));
                }
            })
        });
        let client = Self {
            peer: connected.service.peer().clone(),
            service: Mutex::new(Some(connected.service)),
            lifecycle: StdMutex::new(Lifecycle::default()),
            closing: Notify::new(),
            drained: Notify::new(),
            revision: connected.tools_revision,
            cache: StdMutex::new(None),
            refresh: Mutex::new(()),
            child_pid: connected.child_pid,
            stderr,
            stderr_task: StdMutex::new(stderr_task),
            secrets,
        };
        if let Err(error) = client.list_tools().await {
            let _ = client.close().await;
            let detail = client.stderr_tail().await;
            return Err(if detail.is_empty() {
                error
            } else {
                format!("{error}\n{detail}")
            });
        }
        Ok(client)
    }

    pub fn is_closed(&self) -> bool {
        self.lifecycle
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .closed
            || self.peer.is_transport_closed()
    }

    pub fn child_pid(&self) -> Option<u32> {
        self.child_pid
    }

    pub async fn stderr_tail(&self) -> String {
        self.stderr
            .lock()
            .await
            .iter()
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// A current catalog without I/O. Notifications immediately make it stale.
    pub fn cached_tools(&self) -> Option<Vec<McpTool>> {
        if self.is_closed() {
            return None;
        }
        let cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        cache
            .as_ref()
            .filter(|cache| cache.revision == self.tools_revision())
            .map(|cache| cache.tools.clone())
    }

    pub fn tools_revision(&self) -> u64 {
        self.revision.load(std::sync::atomic::Ordering::Acquire)
    }

    /// Refresh once per revision. Failed refreshes keep the last successful catalog.
    pub async fn list_tools(&self) -> Result<Vec<McpTool>, String> {
        let _refresh = self.refresh.lock().await;
        if let Some(tools) = self.cached_tools() {
            return Ok(tools);
        }
        let revision = self.tools_revision();
        let deadline = tokio::time::Instant::now() + LIST_TOOLS_TIMEOUT;
        let mut cursor = None;
        let mut cursors = HashSet::new();
        let mut tools = Vec::new();
        for _ in 0..100 {
            let request =
                ListToolsRequest::with_param(PaginatedRequestParams::default().with_cursor(cursor));
            let response = self
                .request(
                    ClientRequest::ListToolsRequest(request),
                    deadline.saturating_duration_since(tokio::time::Instant::now()),
                    std::future::pending(),
                )
                .await
                .map_err(|e| e.message)?;
            let ServerResult::ListToolsResult(page) = response else {
                return Err("MCP tools/list returned an unexpected result type".into());
            };
            for tool in page.tools {
                let value = serde_json::to_value(tool).map_err(|e| self.redact(e.to_string()))?;
                tools.push(
                    serde_json::from_value(value)
                        .map_err(|e| self.redact(format!("Invalid MCP tool: {e}")))?,
                );
            }
            cursor = page.next_cursor;
            if cursor.is_none() {
                *self.cache.lock().unwrap_or_else(|e| e.into_inner()) = Some(ToolCache {
                    revision,
                    tools: tools.clone(),
                });
                return Ok(tools);
            }
            if !cursors.insert(cursor.clone()) {
                return Err("MCP tools/list repeated a pagination cursor".into());
            }
        }
        Err("MCP tools/list exceeded 100 pages".into())
    }

    /// Poll to completion so timeout and cancellation can notify the server.
    /// Registry execution drains this future when its agent run stops.
    pub async fn call_tool(
        &self,
        name: &str,
        arguments: Value,
        timeout: Duration,
        cancelled: impl Future<Output = ()> + Send,
    ) -> Result<McpToolCallResult, McpCallFailure> {
        let mut params = CallToolRequestParams::new(name.to_owned());
        if let Value::Object(arguments) = arguments {
            params = params.with_arguments(arguments);
        }
        let result = self
            .request(
                ClientRequest::CallToolRequest(CallToolRequest::new(params)),
                timeout,
                cancelled,
            )
            .await?;
        let ServerResult::CallToolResult(result) = result else {
            return Err(McpCallFailure {
                message: "MCP tools/call returned an unexpected result type".into(),
                replay_safe: false,
            });
        };
        serde_json::to_value(result)
            .map(parse_tool_result)
            .map_err(|e| McpCallFailure {
                message: self.redact(e.to_string()),
                replay_safe: false,
            })
    }

    pub async fn ping(&self, timeout: Duration) -> Result<(), String> {
        self.request(
            ClientRequest::PingRequest(PingRequest::default()),
            timeout,
            std::future::pending(),
        )
        .await
        .map(|_| ())
        .map_err(|e| e.message)
    }

    async fn request(
        &self,
        request: ClientRequest,
        timeout: Duration,
        cancelled: impl Future<Output = ()> + Send,
    ) -> Result<ServerResult, McpCallFailure> {
        let closing = self.closing.notified();
        tokio::pin!(closing);
        closing.as_mut().enable();
        {
            let mut lifecycle = self.lifecycle.lock().unwrap_or_else(|e| e.into_inner());
            if lifecycle.closed {
                return Err(McpCallFailure {
                    message: "MCP connection is closed".into(),
                    replay_safe: true,
                });
            }
            lifecycle.in_flight += 1;
        }
        let _flight = InFlight(self);
        let deadline = tokio::time::sleep(timeout);
        tokio::pin!(deadline, cancelled);
        let mut handle = tokio::select! {
            biased;
            _ = &mut closing => return Err(McpCallFailure { message: "MCP connection is closed".into(), replay_safe: true }),
            _ = &mut cancelled => return Err(McpCallFailure { message: "MCP request cancelled before sending".into(), replay_safe: true }),
            _ = &mut deadline => return Err(McpCallFailure { message: "MCP request timed out before sending".into(), replay_safe: true }),
            result = self.peer.send_cancellable_request(request, PeerRequestOptions::no_options()) => result.map_err(|e| McpCallFailure { message: self.redact(conn::classify_error("request", &e)), replay_safe: true })?,
        };
        // No subscription or progress watcher is registered with no_options().
        let reason = tokio::select! {
            biased;
            response = &mut handle.rx => return response.unwrap_or(Err(rmcp::ServiceError::TransportClosed)).map_err(|e| McpCallFailure { message: self.redact(conn::classify_error("request", &e)), replay_safe: false }),
            _ = &mut closing => "connection closed",
            _ = &mut cancelled => "request cancelled",
            _ = &mut deadline => "request timed out",
        };
        let cancellation =
            tokio::time::timeout(CLEANUP_TIMEOUT, handle.cancel(Some(reason.into()))).await;
        let mut message = format!("MCP {reason}; request outcome is unknown and was not retried");
        match cancellation {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                message.push_str(&format!("; cancellation notification failed: {error}"))
            }
            Err(_) => message.push_str("; cancellation notification timed out"),
        }
        Err(McpCallFailure {
            message: self.redact(message),
            replay_safe: false,
        })
    }

    /// Stop admission, cancel active requests, and await transport and process cleanup.
    /// Cloned `Arc<McpClient>` values do not delay cleanup.
    pub async fn close(&self) -> Result<(), String> {
        {
            self.lifecycle
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .closed = true;
            self.closing.notify_waiters();
        }
        let mut service = self.service.lock().await;
        loop {
            let drained = self.drained.notified();
            tokio::pin!(drained);
            drained.as_mut().enable();
            if self
                .lifecycle
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .in_flight
                == 0
            {
                break;
            }
            drained.await;
        }
        let result = match service.as_mut() {
            Some(service) => match service.close_with_timeout(CLEANUP_TIMEOUT).await {
                Ok(Some(_)) => Ok(()),
                Ok(None) => Err("MCP transport cleanup did not complete within 5 seconds".into()),
                Err(error) => Err(self.redact(format!("MCP close failed: {error}"))),
            },
            None => Ok(()),
        };
        service.take();
        let task = self
            .stderr_task
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        if let Some(task) = task {
            task.abort();
            let _ = task.await;
        }
        result
    }

    fn redact(&self, message: String) -> String {
        redact(message, &self.secrets)
    }
}

impl Drop for McpClient {
    fn drop(&mut self) {
        if let Some(task) = self
            .stderr_task
            .get_mut()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            task.abort();
        }
    }
}

struct InFlight<'a>(&'a McpClient);
impl Drop for InFlight<'_> {
    fn drop(&mut self) {
        let mut lifecycle = self.0.lifecycle.lock().unwrap_or_else(|e| e.into_inner());
        lifecycle.in_flight -= 1;
        if lifecycle.in_flight == 0 {
            self.0.drained.notify_waiters();
        }
    }
}

fn configured_secrets(config: &McpClientConfig) -> Vec<String> {
    let values = match &config.transport {
        McpTransport::Stdio { env, .. } => env,
        McpTransport::StreamableHttp { headers, .. } => headers,
    };
    let mut secrets: Vec<_> = values.values().filter(|v| !v.is_empty()).cloned().collect();
    if let McpTransport::StreamableHttp { url, headers } = &config.transport {
        if !url.trim().is_empty() {
            secrets.push(url.trim().to_owned());
        }
        for (name, value) in headers {
            if !value.trim().is_empty() {
                secrets.push(value.trim().to_owned());
            }
            if name.trim().eq_ignore_ascii_case("authorization") {
                if let Some((_, credential)) = value.trim().split_once(' ') {
                    if !credential.trim().is_empty() {
                        secrets.push(credential.trim().to_owned());
                    }
                }
            }
        }
        if let Ok(url) = reqwest::Url::parse(url) {
            secrets.push(url.as_str().to_owned());
            if !url.username().is_empty() {
                secrets.push(url.username().to_owned());
            }
            if let Some(password) = url.password().filter(|value| !value.is_empty()) {
                secrets.push(password.to_owned());
            }
            secrets.extend(
                url.query_pairs()
                    .filter(|(_, value)| !value.is_empty())
                    .map(|(_, value)| value.into_owned()),
            );
            secrets.extend(
                url.query()
                    .into_iter()
                    .flat_map(|query| query.split('&'))
                    .filter_map(|pair| {
                        let (_, value) = pair.split_once('=')?;
                        (!value.is_empty()).then(|| value.to_owned())
                    }),
            );
        }
    }
    // Longer values first prevent a shorter credential from exposing its suffix.
    secrets.sort_by_key(|value| std::cmp::Reverse(value.len()));
    secrets
}

fn redact(mut message: String, secrets: &[String]) -> String {
    for secret in secrets {
        message = message.replace(secret, "[redacted]");
    }
    message
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transport_debug_and_diagnostics_hide_http_credentials() {
        let config = McpClientConfig {
            transport: McpTransport::StreamableHttp {
                url: "https://username:password@example.test/mcp?api_key=query-secret&token=a%2Fb"
                    .into(),
                headers: HashMap::from([
                    (" Authorization ".into(), "  Bearer bearer-secret  ".into()),
                    ("X-Api-Key".into(), "  header-secret  ".into()),
                ]),
            },
        };
        let secrets = configured_secrets(&config);
        let diagnostic = redact(
            "Bearer bearer-secret bearer-secret header-secret username password query-secret a/b a%2Fb"
                .into(),
            &secrets,
        );
        for secret in [
            "bearer-secret",
            "header-secret",
            "username",
            "password",
            "query-secret",
            "a/b",
            "a%2Fb",
        ] {
            assert!(!diagnostic.contains(secret));
            assert!(!format!("{config:?}").contains(secret));
        }
    }

    #[test]
    fn environment_values_are_redacted_even_with_overlapping_credentials() {
        let config = McpClientConfig {
            transport: McpTransport::Stdio {
                command: "server".into(),
                args: Vec::new(),
                cwd: None,
                env: HashMap::from([
                    ("TOKEN".into(), "secret".into()),
                    ("LONG_TOKEN".into(), "secret-more".into()),
                ]),
            },
        };
        assert_eq!(
            redact("secret-more secret".into(), &configured_secrets(&config)),
            "[redacted] [redacted]"
        );
        assert!(!format!("{config:?}").contains("secret"));
    }
}
