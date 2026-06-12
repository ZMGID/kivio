//! 持久 MCP 连接管理器。
//!
//! 取代旧的"每次调用都 spawn + 握手"一次性连接：每个 server 维护一个
//! 长连接 `McpSession`，stdio 子进程常驻、握手只做一次，按 server_id 挂在
//! `AppState.mcp_sessions` 连接池里。生命周期相关的 reaper / warmup / 退出杀进程
//! 由 main.rs 调度。
//!
//! 关键约束（见 prd 风险段）：
//! - 绝不跨握手 / RPC await 持 `mcp_sessions` 外层池锁；命中即克隆 per-session
//!   `Arc<Mutex<McpSession>>` 后立即释放外层锁。
//! - stdio 子进程 `kill_on_drop(true)`，`StdioConn::Drop` abort reader/stderr task
//!   并 `start_kill()`，退出时再走一遍 `disconnect_all` 兜底，避免孤儿进程。

use std::{
    collections::{HashMap, VecDeque},
    process::Stdio,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use reqwest::{
    header::{HeaderName, HeaderValue, ACCEPT},
    Client,
};
use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Emitter};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{ChildStdin, Command},
    sync::{oneshot, Mutex},
    task::JoinHandle,
    time::timeout,
};

use crate::settings::ChatMcpServer;
use crate::state::AppState;

use super::client;
use super::types::{McpTool, McpToolCallResult};

/// stderr 尾巴最多保留多少行（用于状态面板诊断）。
pub const STDERR_TAIL_LINES: usize = 20;

/// 给前端的 MCP 服务器连接状态。`#[serde(tag = "kind")]` ⇒
/// `{ "kind": "connected" }` / `{ "kind": "error", "message": "..." }`。
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum McpServerState {
    Connecting,
    Connected,
    Error { message: String },
    Disconnected,
}

/// 状态事件发射器。生产代码用 `AppHandle` 发 `mcp-server-state`；测试用 `()` 空实现，
/// 这样核心连接逻辑无需真实 Tauri AppHandle 即可单测。
pub trait McpEventSink {
    fn emit_server_state(&self, server: &ChatMcpServer, state: &McpServerState);
    /// 仅靠 server_id 发 Disconnected（reload / reap 用，可能拿不到完整 server）。
    fn emit_disconnected(&self, server_id: &str);
}

impl McpEventSink for AppHandle {
    fn emit_server_state(&self, server: &ChatMcpServer, state: &McpServerState) {
        let _ = self.emit(
            "mcp-server-state",
            serde_json::json!({
                "serverId": server.id,
                "serverName": server.name,
                "state": state,
            }),
        );
    }

    fn emit_disconnected(&self, server_id: &str) {
        let _ = self.emit(
            "mcp-server-state",
            serde_json::json!({
                "serverId": server_id,
                "state": McpServerState::Disconnected,
            }),
        );
    }
}

impl McpEventSink for () {
    fn emit_server_state(&self, _server: &ChatMcpServer, _state: &McpServerState) {}
    fn emit_disconnected(&self, _server_id: &str) {}
}

/// 单个 MCP 服务器的持久会话。
pub struct McpSession {
    /// ChatMcpServer 序列化指纹：配置变更即重建会话。
    pub config_fingerprint: String,
    pub state: McpServerState,
    pub server_info: Option<Value>,
    pub capabilities: Option<Value>,
    pub tools: Vec<McpTool>,
    /// stderr 尾巴（最近 STDERR_TAIL_LINES 行），用于状态面板。
    pub stderr_tail: Arc<Mutex<VecDeque<String>>>,
    pub last_used: Instant,
    pub handshake_count: u64,
    pub transport: McpTransport,
}

impl McpSession {
    /// 新建占位会话（Connecting，无 transport），插入连接池占位以阻止并发重复握手。
    fn placeholder(fingerprint: String) -> Self {
        Self {
            config_fingerprint: fingerprint,
            state: McpServerState::Connecting,
            server_info: None,
            capabilities: None,
            tools: Vec::new(),
            stderr_tail: Arc::new(Mutex::new(VecDeque::new())),
            last_used: Instant::now(),
            handshake_count: 0,
            transport: McpTransport::None,
        }
    }

    /// 读取 stderr 尾巴快照（拼成多行字符串）。
    pub async fn stderr_tail_text(&self) -> String {
        let tail = self.stderr_tail.lock().await;
        tail.iter().cloned().collect::<Vec<_>>().join("\n")
    }
}

/// 传输层：stdio 持久子进程 or HTTP（按 session_id 复用）。
pub enum McpTransport {
    Stdio(StdioConn),
    Http { session_id: Option<String> },
    /// 占位态：插入连接池但尚未/失败完成握手时使用，避免并发重复握手。
    None,
}

type PendingMap = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, String>>>>>;

/// 持久 stdio 连接：子进程 + 写端 + 后台 reader/stderr task。
/// reader task 循环读 stdout，按 JSON-RPC id 把响应投递给在途请求的 oneshot；
/// 支持并发在途请求。
pub struct StdioConn {
    child: tokio::process::Child,
    stdin: ChildStdin,
    next_id: AtomicU64,
    pending: PendingMap,
    reader_task: JoinHandle<()>,
    stderr_task: JoinHandle<()>,
    timeout: Duration,
}

impl Drop for StdioConn {
    fn drop(&mut self) {
        self.reader_task.abort();
        self.stderr_task.abort();
        // kill_on_drop(true) 已兜底，这里再显式触发一次。
        let _ = self.child.start_kill();
    }
}

impl StdioConn {
    /// liveness 探活：子进程已退出即视为死连接。
    fn is_dead(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(Some(_)) | Err(_))
    }

    /// 发一次 JSON-RPC 请求并等待匹配 id 的响应。
    async fn request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self.pending.lock().await;
            pending.insert(id, tx);
        }
        let mut message = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
        });
        if !params.is_null() {
            message["params"] = params;
        }
        if let Err(err) = self.write_message(&message).await {
            self.pending.lock().await.remove(&id);
            return Err(err);
        }
        match timeout(self.timeout, rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => {
                // reader task 结束（子进程关闭 stdout）时 oneshot sender 被丢弃。
                self.pending.lock().await.remove(&id);
                Err("MCP server closed stdout".to_string())
            }
            Err(_) => {
                self.pending.lock().await.remove(&id);
                Err("MCP stdio read timed out".to_string())
            }
        }
    }

    async fn notify(&mut self, method: &str, params: Value) -> Result<(), String> {
        let mut message = serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
        });
        if !params.is_null() {
            message["params"] = params;
        }
        self.write_message(&message).await
    }

    async fn write_message(&mut self, message: &Value) -> Result<(), String> {
        let line = serde_json::to_string(message).map_err(|err| err.to_string())?;
        timeout(self.timeout, async {
            self.stdin.write_all(line.as_bytes()).await?;
            self.stdin.write_all(b"\n").await?;
            self.stdin.flush().await
        })
        .await
        .map_err(|_| "MCP stdio write timed out".to_string())?
        .map_err(|err| format!("MCP stdio write failed: {err}"))
    }
}

impl AppState {
    /// 当前生效的空闲超时（来自设置 `mcp_idle_timeout_ms`）。
    pub fn mcp_idle_timeout(&self) -> Duration {
        let ms = self.settings_read().chat_tools.mcp_idle_timeout_ms;
        Duration::from_millis(ms)
    }

    /// 取该 server 的工具超时（ms）。
    fn mcp_tool_timeout(&self) -> Duration {
        let ms = self
            .settings_read()
            .chat_tools
            .tool_timeout_ms
            .max(1_000);
        Duration::from_millis(ms)
    }

    /// 命中已连接会话则克隆 Arc 返回；否则建立连接（握手一次）。
    ///
    /// 关键：算 fingerprint / 命中判断在持外层池锁时完成，确定要新建后插入
    /// Connecting 占位、**立即释放外层锁**，再 spawn + initialize（不跨外层锁 await）。
    pub async fn mcp_get_or_connect(
        &self,
        sink: &impl McpEventSink,
        server: &ChatMcpServer,
    ) -> Result<Arc<Mutex<McpSession>>, String> {
        let fingerprint = config_fingerprint(server);

        // 单飞门闩：在持外层池锁时完成「命中已有会话」或「插入 Connecting 占位」二选一，
        // 让并发的第二个调用者一定观察到第一个的占位 Arc（而非各插各的）。
        // 仅当 pool.get 返回 None（或配置已变需重建）时才新建占位并立即插入。
        enum Resolved {
            // 命中已有会话（Connected 或正在 Connecting）：共享其 Arc，锁会话后再判定。
            Existing(Arc<Mutex<McpSession>>),
            // 新建了占位：本调用者负责握手。
            Fresh(Arc<Mutex<McpSession>>),
        }

        let resolved = {
            let mut pool = self.mcp_sessions.lock().await;
            match pool.get(&server.id) {
                Some(existing) => Resolved::Existing(existing.clone()),
                None => {
                    let session =
                        Arc::new(Mutex::new(McpSession::placeholder(fingerprint.clone())));
                    pool.insert(server.id.clone(), session.clone());
                    Resolved::Fresh(session)
                }
            }
        };

        match resolved {
            Resolved::Existing(session) => {
                // 锁会话后重判：可能已被先到的调用者握手成功；或配置已变需重建。
                let mut guard = session.lock().await;
                if guard.config_fingerprint == fingerprint
                    && matches!(guard.state, McpServerState::Connected)
                {
                    drop(guard);
                    return Ok(session);
                }
                if guard.config_fingerprint != fingerprint {
                    // 配置变更：丢弃旧 transport，按新指纹在同一共享会话上重连。
                    guard.config_fingerprint = fingerprint.clone();
                    guard.transport = McpTransport::None;
                    guard.state = McpServerState::Connecting;
                }
                // 共享会话尚未 Connected（Connecting/None/Error）→ 由本调用者握手进它。
                self.connect_session(sink, server, &mut guard).await?;
                drop(guard);
                Ok(session)
            }
            Resolved::Fresh(session) => {
                sink.emit_server_state(server, &McpServerState::Connecting);
                let mut guard = session.lock().await;
                self.connect_session(sink, server, &mut guard).await?;
                drop(guard);
                Ok(session)
            }
        }
    }

    /// 在持有会话锁的前提下完成一次握手（不持外层池锁）。失败时写 Error 状态并返回错误。
    async fn connect_session(
        &self,
        sink: &impl McpEventSink,
        server: &ChatMcpServer,
        guard: &mut McpSession,
    ) -> Result<(), String> {
        match self.mcp_connect_into(server, guard).await {
            Ok(()) => {
                guard.state = McpServerState::Connected;
                guard.handshake_count = guard.handshake_count.saturating_add(1);
                guard.last_used = Instant::now();
                sink.emit_server_state(server, &McpServerState::Connected);
                Ok(())
            }
            Err(err) => {
                let stderr_tail = guard.stderr_tail_text().await;
                let message = if stderr_tail.trim().is_empty() {
                    err
                } else {
                    format!("{err}\n{stderr_tail}")
                };
                guard.state = McpServerState::Error {
                    message: message.clone(),
                };
                guard.transport = McpTransport::None;
                sink.emit_server_state(server, &McpServerState::Error { message: message.clone() });
                Err(message)
            }
        }
    }

    /// 建立 transport 并完成握手，把元数据写入会话（不改 state）。
    async fn mcp_connect_into(
        &self,
        server: &ChatMcpServer,
        session: &mut McpSession,
    ) -> Result<(), String> {
        let timeout_dur = self.mcp_tool_timeout();
        match server.transport.as_str() {
            "streamable_http" => {
                let session_id =
                    http_initialize(&self.http, server, timeout_dur).await?;
                let tools =
                    http_list_tools(&self.http, server, timeout_dur, session_id.as_deref())
                        .await?;
                session.tools = tools;
                session.transport = McpTransport::Http { session_id };
                Ok(())
            }
            _ => {
                let mut conn = spawn_stdio(server, timeout_dur, session.stderr_tail.clone())?;
                let init = conn
                    .request("initialize", client::initialize_params())
                    .await?;
                conn.notify("notifications/initialized", Value::Null).await?;
                session.server_info = init.get("serverInfo").cloned();
                session.capabilities = init.get("capabilities").cloned();
                let list = conn.request("tools/list", Value::Null).await?;
                session.tools = parse_tools(&list)?;
                session.transport = McpTransport::Stdio(conn);
                Ok(())
            }
        }
    }

    /// 调用某个 MCP server 的工具：get-or-connect → 锁会话 → liveness 探活 →
    /// 死则重连一次重试。HTTP 仅 404 / session-not-found 时清 session_id 重试一次。
    pub async fn mcp_call_tool(
        &self,
        sink: &impl McpEventSink,
        server: &ChatMcpServer,
        name: &str,
        arguments: Value,
    ) -> Result<McpToolCallResult, String> {
        let session = self.mcp_get_or_connect(sink, server).await?;
        let mut guard = session.lock().await;

        // liveness：stdio 子进程已死 → 丢弃 transport、重连一次。
        let dead = match &mut guard.transport {
            McpTransport::Stdio(conn) => conn.is_dead(),
            McpTransport::None => true,
            McpTransport::Http { .. } => false,
        };
        if dead {
            guard.transport = McpTransport::None;
            guard.state = McpServerState::Connecting;
            sink.emit_server_state(server, &McpServerState::Connecting);
            if let Err(err) = self.mcp_connect_into(server, &mut guard).await {
                let message = err.clone();
                guard.state = McpServerState::Error {
                    message: message.clone(),
                };
                sink.emit_server_state(server, &McpServerState::Error { message });
                return Err(err);
            }
            guard.state = McpServerState::Connected;
            guard.handshake_count = guard.handshake_count.saturating_add(1);
            sink.emit_server_state(server, &McpServerState::Connected);
        }

        let params = serde_json::json!({ "name": name, "arguments": arguments });
        let result = match &mut guard.transport {
            McpTransport::Stdio(conn) => {
                let value = conn.request("tools/call", params.clone()).await;
                match value {
                    Ok(value) => Ok(client::parse_tool_result(value)),
                    Err(err) => {
                        // 只有连接真死了才丢弃 transport 重连一次重试。慢但健康的工具
                        // （超时）必须把错误透传给调用方，绝不能杀子进程 + 重发同一个
                        // tools/call —— 否则非幂等工具（写文件/付款/发消息）会被静默重复执行。
                        let connection_dead = conn.is_dead() || is_connection_closed_error(&err);
                        if !connection_dead {
                            Err(err)
                        } else {
                            guard.transport = McpTransport::None;
                            self.mcp_connect_into(server, &mut guard).await?;
                            guard.state = McpServerState::Connected;
                            guard.handshake_count = guard.handshake_count.saturating_add(1);
                            sink.emit_server_state(server, &McpServerState::Connected);
                            match &mut guard.transport {
                                McpTransport::Stdio(conn) => {
                                    let value = conn.request("tools/call", params).await?;
                                    Ok(client::parse_tool_result(value))
                                }
                                _ => Err(err),
                            }
                        }
                    }
                }
            }
            McpTransport::Http { session_id } => {
                let timeout_dur = self.mcp_tool_timeout();
                let current = session_id.clone();
                match http_request(&self.http, server, timeout_dur, "tools/call", params.clone(), current.as_deref())
                    .await
                {
                    Ok((value, next_session)) => {
                        *session_id = next_session;
                        Ok(client::parse_tool_result(value))
                    }
                    Err(err) if is_session_expired(&err) => {
                        // 404 / session not found → 清 session_id 重新 initialize 重试一次。
                        let new_session =
                            http_initialize(&self.http, server, timeout_dur).await?;
                        let (value, next_session) = http_request(
                            &self.http,
                            server,
                            timeout_dur,
                            "tools/call",
                            params,
                            new_session.as_deref(),
                        )
                        .await?;
                        *session_id = next_session.or(new_session);
                        Ok(client::parse_tool_result(value))
                    }
                    Err(err) => Err(err),
                }
            }
            McpTransport::None => Err("MCP transport unavailable".to_string()),
        };

        if result.is_ok() {
            guard.last_used = Instant::now();
        }
        result
    }

    /// 返回某个 server 的工具列表（持久会话）。供 list_enabled_tool_defs 复用。
    pub async fn mcp_list_tools(
        &self,
        sink: &impl McpEventSink,
        server: &ChatMcpServer,
    ) -> Result<Vec<McpTool>, String> {
        let session = self.mcp_get_or_connect(sink, server).await?;
        let mut guard = session.lock().await;
        guard.last_used = Instant::now();
        Ok(guard.tools.clone())
    }

    /// 读取某个 server 的状态快照（给状态命令）。无会话 ⇒ Disconnected。
    pub async fn mcp_server_state(&self, server_id: &str) -> McpServerStatusSnapshot {
        let session = {
            let pool = self.mcp_sessions.lock().await;
            pool.get(server_id).cloned()
        };
        match session {
            Some(session) => {
                let guard = session.lock().await;
                McpServerStatusSnapshot {
                    server_id: server_id.to_string(),
                    state: guard.state.clone(),
                    handshake_count: guard.handshake_count,
                    stderr_tail: guard.stderr_tail_text().await,
                }
            }
            None => McpServerStatusSnapshot {
                server_id: server_id.to_string(),
                state: McpServerState::Disconnected,
                handshake_count: 0,
                stderr_tail: String::new(),
            },
        }
    }

    /// 主动丢弃某个 server 的会话（重连按钮用），下次调用透明重连。
    pub async fn mcp_reload_server(&self, sink: &impl McpEventSink, server_id: &str) {
        let removed = {
            let mut pool = self.mcp_sessions.lock().await;
            pool.remove(server_id)
        };
        if let Some(session) = removed {
            let mut guard = session.lock().await;
            guard.transport = McpTransport::None;
            guard.state = McpServerState::Disconnected;
        }
        sink.emit_disconnected(server_id);
    }

    /// 空闲回收：移除 last_used 超过 idle_timeout 的会话。锁内只收集+移除，无 await。
    /// 返回被回收的 server_id（供发 Disconnected 事件）。
    pub async fn mcp_reap_idle(&self, idle_timeout: Duration) -> Vec<(String, Arc<Mutex<McpSession>>)> {
        let now = Instant::now();
        let mut evicted = Vec::new();
        // 先并发拿每个会话的 last_used 需要锁会话；为避免锁内 await，这里改为：
        // 收集所有 (id, Arc)，释放池锁后逐个判断。
        let candidates: Vec<(String, Arc<Mutex<McpSession>>)> = {
            let pool = self.mcp_sessions.lock().await;
            pool.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
        };
        let mut expired_ids = Vec::new();
        for (id, session) in &candidates {
            let guard = session.lock().await;
            if now.duration_since(guard.last_used) > idle_timeout {
                expired_ids.push(id.clone());
            }
        }
        if expired_ids.is_empty() {
            return evicted;
        }
        {
            let mut pool = self.mcp_sessions.lock().await;
            for id in &expired_ids {
                if let Some(session) = pool.remove(id) {
                    evicted.push((id.clone(), session));
                }
            }
        }
        for (_, session) in &evicted {
            let mut guard = session.lock().await;
            guard.transport = McpTransport::None;
            guard.state = McpServerState::Disconnected;
        }
        evicted
    }

    /// 排干连接池：每个会话 Drop transport 触发 abort task + kill 子进程。退出钩子用。
    pub async fn mcp_disconnect_all(&self) {
        let drained: Vec<(String, Arc<Mutex<McpSession>>)> = {
            let mut pool = self.mcp_sessions.lock().await;
            pool.drain().collect()
        };
        for (_, session) in drained {
            let mut guard = session.lock().await;
            guard.transport = McpTransport::None;
            guard.state = McpServerState::Disconnected;
        }
    }
}

/// 状态命令返回给前端的快照。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServerStatusSnapshot {
    pub server_id: String,
    pub state: McpServerState,
    pub handshake_count: u64,
    pub stderr_tail: String,
}

/// ChatMcpServer 配置指纹：序列化后做稳定哈希，配置变更即重建会话。
pub fn config_fingerprint(server: &ChatMcpServer) -> String {
    serde_json::to_string(server).unwrap_or_else(|_| format!("{}:{}", server.id, server.command))
}

fn is_session_expired(err: &str) -> bool {
    let lower = err.to_ascii_lowercase();
    err.contains("404") || lower.contains("session not found") || lower.contains("session-not-found")
}

/// stdio request 错误是否表示连接已关闭（reader task 结束、子进程关 stdout）。
/// 用于区分「连接死了应重连」与「慢但健康的工具读/写超时应透传」。
fn is_connection_closed_error(err: &str) -> bool {
    err.contains("MCP server closed stdout")
}

fn parse_tools(value: &Value) -> Result<Vec<McpTool>, String> {
    let tools = value
        .get("tools")
        .cloned()
        .unwrap_or_else(|| Value::Array(Vec::new()));
    serde_json::from_value(tools).map_err(|err| format!("MCP tools/list parse failed: {err}"))
}

/// spawn stdio 子进程并启动 reader / stderr 后台 task。
fn spawn_stdio(
    server: &ChatMcpServer,
    timeout_dur: Duration,
    stderr_tail: Arc<Mutex<VecDeque<String>>>,
) -> Result<StdioConn, String> {
    if server.command.trim().is_empty() {
        return Err("MCP server command is empty".to_string());
    }
    let mut command = Command::new(&server.command);
    command.args(&server.args);
    if let Some(cwd) = server.cwd.as_deref().filter(|cwd| !cwd.trim().is_empty()) {
        command.current_dir(cwd);
    }
    command.envs(client::clean_env(&server.env));
    command.stdin(Stdio::piped());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    command.kill_on_drop(true);

    let mut child = command
        .spawn()
        .map_err(|err| format!("Failed to start MCP server {}: {err}", server.name))?;
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| "MCP server stdin unavailable".to_string())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "MCP server stdout unavailable".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "MCP server stderr unavailable".to_string())?;

    let pending: PendingMap = Arc::new(Mutex::new(HashMap::new()));

    // reader task：循环读 stdout 行，按 JSON-RPC id 投递给在途请求的 oneshot。
    let reader_pending = pending.clone();
    let reader_task = tokio::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        loop {
            match lines.next_line().await {
                Ok(Some(line)) => {
                    let value: Value = match serde_json::from_str(&line) {
                        Ok(value) => value,
                        Err(_) => continue,
                    };
                    // 无 id 的通知 / 进度消息：忽略（保留旧 read_response 逻辑）。
                    let Some(id) = value.get("id").and_then(|id| id.as_u64()) else {
                        continue;
                    };
                    let sender = {
                        let mut pending = reader_pending.lock().await;
                        pending.remove(&id)
                    };
                    if let Some(sender) = sender {
                        let outcome = if let Some(error) = value.get("error") {
                            Err(format!(
                                "MCP error: {}",
                                client::compact_json(error, 500)
                            ))
                        } else {
                            Ok(value.get("result").cloned().unwrap_or(Value::Null))
                        };
                        let _ = sender.send(outcome);
                    }
                }
                // EOF 或读错误 → 子进程关闭 stdout，结束 reader；在途请求的 oneshot
                // 在此被丢弃，request 侧收到 RecvError 报 "closed stdout"。
                Ok(None) | Err(_) => break,
            }
        }
    });

    // stderr task：把 stderr 尾巴收进环形缓冲（最多 STDERR_TAIL_LINES 行）。
    let stderr_task = tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let mut tail = stderr_tail.lock().await;
            if tail.len() >= STDERR_TAIL_LINES {
                tail.pop_front();
            }
            tail.push_back(line);
        }
    });

    Ok(StdioConn {
        child,
        stdin,
        next_id: AtomicU64::new(1),
        pending,
        reader_task,
        stderr_task,
        timeout: timeout_dur,
    })
}

async fn http_initialize(
    http: &Client,
    server: &ChatMcpServer,
    timeout_dur: Duration,
) -> Result<Option<String>, String> {
    let (_value, session_id) = http_request(
        http,
        server,
        timeout_dur,
        "initialize",
        client::initialize_params(),
        None,
    )
    .await?;
    http_notify(
        http,
        server,
        timeout_dur,
        "notifications/initialized",
        Value::Null,
        session_id.as_deref(),
    )
    .await?;
    Ok(session_id)
}

async fn http_list_tools(
    http: &Client,
    server: &ChatMcpServer,
    timeout_dur: Duration,
    session_id: Option<&str>,
) -> Result<Vec<McpTool>, String> {
    let (value, _) = http_request(http, server, timeout_dur, "tools/list", Value::Null, session_id)
        .await?;
    parse_tools(&value)
}

async fn http_notify(
    http: &Client,
    server: &ChatMcpServer,
    timeout_dur: Duration,
    method: &str,
    params: Value,
    session_id: Option<&str>,
) -> Result<(), String> {
    let mut message = serde_json::json!({
        "jsonrpc": "2.0",
        "method": method,
    });
    if !params.is_null() {
        message["params"] = params;
    }
    let response = http_post(http, server, timeout_dur, message, session_id).await?;
    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        return Err(format!(
            "MCP HTTP notify failed {}: {}",
            status.as_u16(),
            text.chars().take(500).collect::<String>()
        ));
    }
    Ok(())
}

/// 单次 HTTP JSON-RPC 请求；返回 (result, next_session_id)。
async fn http_request(
    http: &Client,
    server: &ChatMcpServer,
    timeout_dur: Duration,
    method: &str,
    params: Value,
    session_id: Option<&str>,
) -> Result<(Value, Option<String>), String> {
    let id = uuid::Uuid::new_v4().to_string();
    let mut message = serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
    });
    if !params.is_null() {
        message["params"] = params;
    }
    let response = http_post(http, server, timeout_dur, message, session_id).await?;
    let next_session_id = response
        .headers()
        .get("mcp-session-id")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.to_string())
        .or_else(|| session_id.map(|value| value.to_string()));
    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        return Err(format!(
            "MCP HTTP request failed {}: {}",
            status.as_u16(),
            text.chars().take(500).collect::<String>()
        ));
    }
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let value = if content_type.contains("text/event-stream") {
        timeout(timeout_dur, client::read_sse_json_rpc_response(response, &id))
            .await
            .map_err(|_| "MCP HTTP SSE read timed out".to_string())??
    } else {
        let text = timeout(timeout_dur, response.text())
            .await
            .map_err(|_| "MCP HTTP read body timed out".to_string())?
            .map_err(|err| format!("MCP HTTP read body failed: {err}"))?;
        if text.trim_start().starts_with("event:") || text.trim_start().starts_with("data:") {
            client::parse_sse_json_rpc(&text, &id)?
        } else if text.trim().is_empty() {
            Value::Null
        } else {
            serde_json::from_str(&text).map_err(|err| {
                format!(
                    "MCP HTTP parse JSON failed: {} (body: {})",
                    err,
                    text.chars().take(500).collect::<String>()
                )
            })?
        }
    };
    if let Some(error) = value.get("error") {
        return Err(format!(
            "MCP HTTP error: {}",
            client::compact_json(error, 500)
        ));
    }
    Ok((
        value.get("result").cloned().unwrap_or(Value::Null),
        next_session_id,
    ))
}

async fn http_post(
    http: &Client,
    server: &ChatMcpServer,
    timeout_dur: Duration,
    message: Value,
    session_id: Option<&str>,
) -> Result<reqwest::Response, String> {
    if server.url.trim().is_empty() {
        return Err("MCP HTTP server URL is empty".to_string());
    }
    let mut headers = client::http_headers(&server.headers)?;
    headers.insert(
        ACCEPT,
        HeaderValue::from_static("application/json, text/event-stream"),
    );
    headers.insert(
        HeaderName::from_static("mcp-protocol-version"),
        HeaderValue::from_static(client::MCP_PROTOCOL_VERSION_HEADER),
    );
    if let Some(session_id) = session_id.filter(|value| !value.trim().is_empty()) {
        let value = HeaderValue::from_str(session_id)
            .map_err(|err| format!("Invalid MCP session id header: {err}"))?;
        headers.insert(HeaderName::from_static("mcp-session-id"), value);
    }
    timeout(
        timeout_dur,
        http.post(server.url.clone())
            .headers(headers)
            .json(&message)
            .send(),
    )
    .await
    .map_err(|_| "MCP HTTP request timed out".to_string())?
    .map_err(|err| format!("MCP HTTP request failed: {err}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::test_app_state;

    #[test]
    fn config_fingerprint_changes_with_config() {
        let mut server = stdio_server("echo", &[]);
        let a = config_fingerprint(&server);
        server.args = vec!["--flag".to_string()];
        let b = config_fingerprint(&server);
        assert_ne!(a, b);
        // 同配置稳定。
        assert_eq!(b, config_fingerprint(&server));
    }

    #[test]
    fn is_session_expired_matches_404_and_session_not_found() {
        assert!(is_session_expired("MCP HTTP request failed 404: not found"));
        assert!(is_session_expired("error: Session not found"));
        assert!(!is_session_expired("MCP HTTP request failed 500: boom"));
    }

    fn stdio_server(command: &str, args: &[&str]) -> ChatMcpServer {
        ChatMcpServer {
            id: "test-stdio".to_string(),
            name: "Test Stdio".to_string(),
            enabled: true,
            transport: "stdio".to_string(),
            url: String::new(),
            command: command.to_string(),
            args: args.iter().map(|a| a.to_string()).collect(),
            env: std::collections::HashMap::new(),
            headers: std::collections::HashMap::new(),
            cwd: None,
            enabled_tools: Vec::new(),
        }
    }

    fn http_server(url: String) -> ChatMcpServer {
        ChatMcpServer {
            id: "test-http".to_string(),
            name: "Test HTTP".to_string(),
            enabled: true,
            transport: "streamable_http".to_string(),
            url,
            command: String::new(),
            args: Vec::new(),
            env: std::collections::HashMap::new(),
            headers: std::collections::HashMap::new(),
            cwd: None,
            enabled_tools: Vec::new(),
        }
    }

    /// fake HTTP MCP server：第一次 tools/call 返回 `first_call_status`（>=400 视为失败），
    /// 之后 tools/call 正常回 echo。initialize/tools/list 始终正常。
    async fn spawn_test_http_mcp_server(first_call_status: u16) -> String {
        use std::sync::atomic::{AtomicU64, Ordering};
        use std::sync::Arc;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        let call_count = Arc::new(AtomicU64::new(0));
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                let call_count = call_count.clone();
                tokio::spawn(async move {
                    let mut buffer = vec![0_u8; 8192];
                    let mut read = 0_usize;
                    loop {
                        let Ok(n) = stream.read(&mut buffer[read..]).await else {
                            return;
                        };
                        if n == 0 {
                            return;
                        }
                        read += n;
                        let request = String::from_utf8_lossy(&buffer[..read]);
                        let Some(header_end) = request.find("\r\n\r\n") else {
                            continue;
                        };
                        let content_length = request
                            .lines()
                            .find_map(|line| {
                                line.strip_prefix("Content-Length:")
                                    .or_else(|| line.strip_prefix("content-length:"))
                                    .and_then(|value| value.trim().parse::<usize>().ok())
                            })
                            .unwrap_or(0);
                        if read < header_end + 4 + content_length {
                            continue;
                        }
                        let body = &request[header_end + 4..header_end + 4 + content_length];
                        let message: Value = serde_json::from_str(body).expect("json");
                        let method = message
                            .get("method")
                            .and_then(|m| m.as_str())
                            .unwrap_or_default();
                        let id = message.get("id").cloned().unwrap_or(Value::Null);

                        // tools/call 第一次按配置返回错误状态。
                        if method == "tools/call" {
                            let nth = call_count.fetch_add(1, Ordering::SeqCst) + 1;
                            if nth == 1 && first_call_status >= 400 {
                                let raw = format!(
                                    "HTTP/1.1 {} ERR\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                                    first_call_status
                                );
                                let _ = stream.write_all(raw.as_bytes()).await;
                                let _ = stream.shutdown().await;
                                return;
                            }
                        }

                        let response = match method {
                            "initialize" => serde_json::json!({
                                "jsonrpc":"2.0","id":id,
                                "result":{"protocolVersion":"2025-06-18","capabilities":{"tools":{}},"serverInfo":{"name":"fake","version":"1.0.0"}}
                            }),
                            "tools/list" => serde_json::json!({
                                "jsonrpc":"2.0","id":id,
                                "result":{"tools":[{"name":"echo","description":"Echo","inputSchema":{"type":"object"}}]}
                            }),
                            "tools/call" => serde_json::json!({
                                "jsonrpc":"2.0","id":id,
                                "result":{"content":[{"type":"text","text":"echo: ok"}]}
                            }),
                            _ => serde_json::json!({"jsonrpc":"2.0","id":id,"result":{}}),
                        };
                        let payload = if message.get("id").is_some() {
                            response.to_string()
                        } else {
                            String::new()
                        };
                        let raw = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nMcp-Session-Id: sess-1\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                            payload.len(),
                            payload,
                        );
                        let _ = stream.write_all(raw.as_bytes()).await;
                        let _ = stream.shutdown().await;
                        return;
                    }
                });
            }
        });
        format!("http://{addr}/mcp")
    }

    #[tokio::test]
    async fn http_reconnect_only_on_404() {
        // 404 → 清 session 重 initialize 重试 → 成功。
        let url = spawn_test_http_mcp_server(404).await;
        let state = test_app_state();
        let server = http_server(url);
        let result = state
            .mcp_call_tool(&(), &server, "echo", serde_json::json!({}))
            .await
            .expect("404 should transparently reconnect and retry");
        assert_eq!(result.content, "echo: ok");
        state.mcp_disconnect_all().await;
    }

    #[tokio::test]
    async fn http_500_does_not_reconnect() {
        // 500 不是 session 过期 → 透传错误，不重试。
        let url = spawn_test_http_mcp_server(500).await;
        let state = test_app_state();
        let server = http_server(url);
        let err = state
            .mcp_call_tool(&(), &server, "echo", serde_json::json!({}))
            .await
            .expect_err("500 should surface as error");
        assert!(err.contains("500"), "got: {err}");
        state.mcp_disconnect_all().await;
    }

    /// fake HTTP MCP server：始终 200，计数收到的 initialize 次数（用于断言会话复用）。
    /// 返回 (url, initialize_count)。
    async fn spawn_counting_http_mcp_server() -> (String, std::sync::Arc<std::sync::atomic::AtomicU64>) {
        use std::sync::atomic::{AtomicU64, Ordering};
        use std::sync::Arc;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        let init_count = Arc::new(AtomicU64::new(0));
        let init_count_server = init_count.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                let init_count = init_count_server.clone();
                tokio::spawn(async move {
                    let mut buffer = vec![0_u8; 8192];
                    let mut read = 0_usize;
                    loop {
                        let Ok(n) = stream.read(&mut buffer[read..]).await else {
                            return;
                        };
                        if n == 0 {
                            return;
                        }
                        read += n;
                        let request = String::from_utf8_lossy(&buffer[..read]);
                        let Some(header_end) = request.find("\r\n\r\n") else {
                            continue;
                        };
                        let content_length = request
                            .lines()
                            .find_map(|line| {
                                line.strip_prefix("Content-Length:")
                                    .or_else(|| line.strip_prefix("content-length:"))
                                    .and_then(|value| value.trim().parse::<usize>().ok())
                            })
                            .unwrap_or(0);
                        if read < header_end + 4 + content_length {
                            continue;
                        }
                        let body = &request[header_end + 4..header_end + 4 + content_length];
                        let message: Value = serde_json::from_str(body).expect("json");
                        let method = message
                            .get("method")
                            .and_then(|m| m.as_str())
                            .unwrap_or_default();
                        let id = message.get("id").cloned().unwrap_or(Value::Null);

                        if method == "initialize" {
                            init_count.fetch_add(1, Ordering::SeqCst);
                        }

                        let response = match method {
                            "initialize" => serde_json::json!({
                                "jsonrpc":"2.0","id":id,
                                "result":{"protocolVersion":"2025-06-18","capabilities":{"tools":{}},"serverInfo":{"name":"fake","version":"1.0.0"}}
                            }),
                            "tools/list" => serde_json::json!({
                                "jsonrpc":"2.0","id":id,
                                "result":{"tools":[{"name":"echo","description":"Echo","inputSchema":{"type":"object"}}]}
                            }),
                            "tools/call" => serde_json::json!({
                                "jsonrpc":"2.0","id":id,
                                "result":{"content":[{"type":"text","text":"echo: ok"}]}
                            }),
                            _ => serde_json::json!({"jsonrpc":"2.0","id":id,"result":{}}),
                        };
                        let payload = if message.get("id").is_some() {
                            response.to_string()
                        } else {
                            String::new()
                        };
                        let raw = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nMcp-Session-Id: sess-1\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                            payload.len(),
                            payload,
                        );
                        let _ = stream.write_all(raw.as_bytes()).await;
                        let _ = stream.shutdown().await;
                        return;
                    }
                });
            }
        });
        (format!("http://{addr}/mcp"), init_count)
    }

    #[tokio::test]
    async fn http_two_successful_calls_reuse_one_initialize() {
        // FIX 7: 两次成功 (200) HTTP 调用复用同一 session_id，全程只 initialize 一次。
        use std::sync::atomic::Ordering;
        let (url, init_count) = spawn_counting_http_mcp_server().await;
        let state = test_app_state();
        let server = http_server(url);

        let first = state
            .mcp_call_tool(&(), &server, "echo", serde_json::json!({}))
            .await
            .expect("first call ok");
        assert_eq!(first.content, "echo: ok");
        let second = state
            .mcp_call_tool(&(), &server, "echo", serde_json::json!({}))
            .await
            .expect("second call ok");
        assert_eq!(second.content, "echo: ok");

        assert_eq!(
            init_count.load(Ordering::SeqCst),
            1,
            "two successful HTTP calls must share a single initialize"
        );
        state.mcp_disconnect_all().await;
    }

    #[cfg(unix)]
    mod stdio {
        use super::*;
        use std::io::Write;

        /// 写一个 fake stdio MCP server python 脚本到临时文件，返回脚本路径。
        /// 协议：逐行读 JSON-RPC；initialize/tools/list/tools/call 各自回包；
        /// 无 id 的通知忽略。若设置 `KIVIO_DIE_AFTER_CALL=N`，在第 N 次 tools/call 回包后退出，
        /// 用于模拟子进程死亡 → 透明重连。
        /// `KIVIO_DELAY_CALL_MS=N`：响应 tools/call 前先 sleep N 毫秒（模拟慢但健康的工具）。
        /// `KIVIO_CALL_MARKER=path`：每次执行 tools/call 时往该文件追加一行（统计实际执行次数）。
        fn write_fake_server() -> std::path::PathBuf {
            let script = r#"#!/usr/bin/env python3
import sys, json, os, time
die_after = int(os.environ.get("KIVIO_DIE_AFTER_CALL", "0"))
delay_ms = int(os.environ.get("KIVIO_DELAY_CALL_MS", "0"))
marker = os.environ.get("KIVIO_CALL_MARKER", "")
calls = 0
while True:
    line = sys.stdin.readline()
    if not line:
        break
    line = line.strip()
    if not line:
        continue
    try:
        msg = json.loads(line)
    except Exception:
        continue
    mid = msg.get("id")
    method = msg.get("method")
    if mid is None:
        # notification, ignore
        continue
    if method == "initialize":
        resp = {"jsonrpc":"2.0","id":mid,"result":{"protocolVersion":"2025-06-18","capabilities":{"tools":{}},"serverInfo":{"name":"fake","version":"1.0.0"}}}
    elif method == "tools/list":
        resp = {"jsonrpc":"2.0","id":mid,"result":{"tools":[{"name":"echo","description":"Echo","inputSchema":{"type":"object","properties":{"text":{"type":"string"}}}}]}}
    elif method == "tools/call":
        calls += 1
        if marker:
            with open(marker, "a") as f:
                f.write("call\n")
        text = ""
        try:
            text = msg["params"]["arguments"].get("text","")
        except Exception:
            text = ""
        if delay_ms:
            time.sleep(delay_ms / 1000.0)
        resp = {"jsonrpc":"2.0","id":mid,"result":{"content":[{"type":"text","text":"echo: "+str(text)}]}}
        sys.stdout.write(json.dumps(resp)+"\n")
        sys.stdout.flush()
        if die_after and calls >= die_after:
            sys.exit(0)
        continue
    else:
        resp = {"jsonrpc":"2.0","id":mid,"result":{}}
    sys.stdout.write(json.dumps(resp)+"\n")
    sys.stdout.flush()
"#;
            let mut path = std::env::temp_dir();
            path.push(format!("kivio-fake-mcp-{}.py", uuid::Uuid::new_v4()));
            let mut file = std::fs::File::create(&path).expect("create fake server");
            file.write_all(script.as_bytes()).expect("write fake server");
            path
        }

        fn python_server(script: &std::path::Path) -> ChatMcpServer {
            super::stdio_server("python3", &["-u", script.to_str().unwrap()])
        }

        #[tokio::test]
        async fn ten_calls_one_handshake() {
            let script = write_fake_server();
            let state = test_app_state();
            let server = python_server(&script);

            for i in 0..10 {
                let result = state
                    .mcp_call_tool(&(), &server, "echo", serde_json::json!({ "text": i }))
                    .await
                    .expect("call should succeed");
                assert_eq!(result.content, format!("echo: {i}"));
            }

            let handshake_count = {
                let pool = state.mcp_sessions.lock().await;
                let session = pool.get(&server.id).expect("session present").clone();
                drop(pool);
                let guard = session.lock().await;
                guard.handshake_count
            };
            assert_eq!(handshake_count, 1, "10 calls must share 1 handshake");
            state.mcp_disconnect_all().await;
            let _ = std::fs::remove_file(&script);
        }

        #[tokio::test]
        async fn timeout_on_healthy_child_does_not_kill_or_reexecute() {
            // FIX 1: 一个慢但健康的工具超过 tool_timeout_ms ⇒ request 返回 "read timed out"。
            // 必须把错误透传，绝不杀健康子进程、不重连、不重发同一个 tools/call
            // （否则非幂等工具会被静默重复执行）。
            let script = write_fake_server();
            let state = test_app_state();
            // 注入最小工具超时（1s，受 .max(1000) 约束）；server 延迟 2.5s 远超之。
            state.settings_write().chat_tools.tool_timeout_ms = 1_000;

            let mut marker = std::env::temp_dir();
            marker.push(format!("kivio-fake-mcp-marker-{}.txt", uuid::Uuid::new_v4()));

            let mut server = python_server(&script);
            server
                .env
                .insert("KIVIO_DELAY_CALL_MS".to_string(), "2500".to_string());
            server.env.insert(
                "KIVIO_CALL_MARKER".to_string(),
                marker.to_string_lossy().into_owned(),
            );

            let err = state
                .mcp_call_tool(&(), &server, "echo", serde_json::json!({ "text": "slow" }))
                .await
                .expect_err("slow healthy tool should surface a timeout error");
            assert!(
                err.contains("timed out"),
                "expected a timeout error, got: {err}"
            );

            // 握手仍为 1（未重连），子进程仍存活。
            let (handshake_count, pid) = {
                let pool = state.mcp_sessions.lock().await;
                let session = pool.get(&server.id).expect("session present").clone();
                drop(pool);
                let mut guard = session.lock().await;
                let alive = match &mut guard.transport {
                    McpTransport::Stdio(conn) => !conn.is_dead() && conn.child.id().is_some(),
                    _ => false,
                };
                assert!(alive, "healthy child must not be killed by a timeout");
                let pid = match &guard.transport {
                    McpTransport::Stdio(conn) => conn.child.id(),
                    _ => None,
                };
                (guard.handshake_count, pid)
            };
            assert_eq!(
                handshake_count, 1,
                "timeout must not trigger a reconnect/re-handshake"
            );
            assert!(pid.is_some(), "child pid should still be present");

            // 给延迟的 tools/call 充足时间真正执行完一次（验证只执行一次，没被重发）。
            tokio::time::sleep(Duration::from_millis(3_000)).await;
            let marker_lines = std::fs::read_to_string(&marker).unwrap_or_default();
            let executed = marker_lines.lines().filter(|l| *l == "call").count();
            assert_eq!(
                executed, 1,
                "the tool body must run exactly once (no silent re-execution)"
            );

            state.mcp_disconnect_all().await;
            let _ = std::fs::remove_file(&script);
            let _ = std::fs::remove_file(&marker);
        }

        #[tokio::test]
        async fn concurrent_get_or_connect_share_one_handshake() {
            // FIX 2: 同一 server_id 的两个并发 get_or_connect（无已连接会话）必须收敛到
            // 单飞门闩，只做一次握手、只有一个池条目。
            let script = write_fake_server();
            let state = std::sync::Arc::new(test_app_state());
            let server = python_server(&script);

            let s1 = state.clone();
            let srv1 = server.clone();
            let s2 = state.clone();
            let srv2 = server.clone();
            let h1 = tokio::spawn(async move { s1.mcp_get_or_connect(&(), &srv1).await.map(|_| ()) });
            let h2 = tokio::spawn(async move { s2.mcp_get_or_connect(&(), &srv2).await.map(|_| ()) });
            let (r1, r2) = tokio::join!(h1, h2);
            r1.unwrap().expect("connect one ok");
            r2.unwrap().expect("connect two ok");

            let (entries, handshake_count) = {
                let pool = state.mcp_sessions.lock().await;
                let entries = pool.len();
                let session = pool.get(&server.id).expect("session present").clone();
                drop(pool);
                let guard = session.lock().await;
                (entries, guard.handshake_count)
            };
            assert_eq!(entries, 1, "exactly one pool entry for the server");
            assert_eq!(
                handshake_count, 1,
                "two concurrent connects must share a single handshake"
            );

            state.mcp_disconnect_all().await;
            let _ = std::fs::remove_file(&script);
        }

        #[tokio::test]
        async fn liveness_reconnect_on_dead_child() {
            let script = write_fake_server();
            let state = test_app_state();
            let mut server = python_server(&script);
            // server 在第 1 次 tools/call 后退出 → 第 2 次调用探活发现死连接 → 透明重连。
            server
                .env
                .insert("KIVIO_DIE_AFTER_CALL".to_string(), "1".to_string());

            let first = state
                .mcp_call_tool(&(), &server, "echo", serde_json::json!({ "text": "a" }))
                .await
                .expect("first call ok");
            assert_eq!(first.content, "echo: a");

            // 给子进程一点时间真正退出。
            tokio::time::sleep(Duration::from_millis(200)).await;

            let second = state
                .mcp_call_tool(&(), &server, "echo", serde_json::json!({ "text": "b" }))
                .await
                .expect("second call should transparently reconnect");
            assert_eq!(second.content, "echo: b");

            let handshake_count = {
                let pool = state.mcp_sessions.lock().await;
                let session = pool.get(&server.id).expect("session present").clone();
                drop(pool);
                let guard = session.lock().await;
                guard.handshake_count
            };
            assert_eq!(
                handshake_count, 2,
                "dead child should trigger exactly one reconnect"
            );
            state.mcp_disconnect_all().await;
            let _ = std::fs::remove_file(&script);
        }

        #[tokio::test]
        async fn concurrent_in_flight_requests_match_by_id() {
            // 同一会话上并发两个 tools/call，验证 reader task 按 id 正确关联。
            let script = write_fake_server();
            let state = std::sync::Arc::new(test_app_state());
            let server = python_server(&script);

            // 先建立连接（避免两个并发同时握手）。
            state
                .mcp_call_tool(&(), &server, "echo", serde_json::json!({ "text": "warm" }))
                .await
                .expect("warmup ok");

            let s1 = state.clone();
            let srv1 = server.clone();
            let s2 = state.clone();
            let srv2 = server.clone();
            let h1 = tokio::spawn(async move {
                s1.mcp_call_tool(&(), &srv1, "echo", serde_json::json!({ "text": "one" }))
                    .await
            });
            let h2 = tokio::spawn(async move {
                s2.mcp_call_tool(&(), &srv2, "echo", serde_json::json!({ "text": "two" }))
                    .await
            });
            let (r1, r2) = tokio::join!(h1, h2);
            let r1 = r1.unwrap().expect("call one ok");
            let r2 = r2.unwrap().expect("call two ok");
            assert_eq!(r1.content, "echo: one");
            assert_eq!(r2.content, "echo: two");

            state.mcp_disconnect_all().await;
            let _ = std::fs::remove_file(&script);
        }

        #[tokio::test]
        async fn idle_reap_evicts_and_reconnects() {
            let script = write_fake_server();
            let state = test_app_state();
            let server = python_server(&script);

            state
                .mcp_call_tool(&(), &server, "echo", serde_json::json!({ "text": "x" }))
                .await
                .expect("call ok");
            {
                let pool = state.mcp_sessions.lock().await;
                assert!(pool.contains_key(&server.id));
            }

            // 注入极小空闲超时 → 立即过期回收。
            tokio::time::sleep(Duration::from_millis(20)).await;
            let evicted = state.mcp_reap_idle(Duration::from_millis(1)).await;
            assert_eq!(evicted.len(), 1);
            {
                let pool = state.mcp_sessions.lock().await;
                assert!(!pool.contains_key(&server.id), "session should be reaped");
            }

            // 回收后下次调用透明重连。
            let again = state
                .mcp_call_tool(&(), &server, "echo", serde_json::json!({ "text": "y" }))
                .await
                .expect("reconnect after reap ok");
            assert_eq!(again.content, "echo: y");

            // 回收后的重连必须是一次全新握手。回收会把旧会话整个移出连接池
            // （上面已断言 !contains_key），下次调用新建一个全新会话并握手一次 ⇒
            // 新会话 handshake_count == 1。这证明重连走了全新握手而非复用旧连接。
            let handshake_count = {
                let pool = state.mcp_sessions.lock().await;
                let session = pool.get(&server.id).expect("session present").clone();
                drop(pool);
                let guard = session.lock().await;
                guard.handshake_count
            };
            assert_eq!(
                handshake_count, 1,
                "reconnect after reap must build a fresh session with exactly one handshake"
            );

            state.mcp_disconnect_all().await;
            let _ = std::fs::remove_file(&script);
        }

        #[tokio::test]
        async fn disconnect_all_kills_children() {
            let script = write_fake_server();
            let state = test_app_state();
            let server = python_server(&script);

            state
                .mcp_call_tool(&(), &server, "echo", serde_json::json!({ "text": "x" }))
                .await
                .expect("call ok");
            // 记录子进程 pid。
            let pid = {
                let pool = state.mcp_sessions.lock().await;
                let session = pool.get(&server.id).unwrap().clone();
                drop(pool);
                let guard = session.lock().await;
                match &guard.transport {
                    McpTransport::Stdio(conn) => conn.child.id(),
                    _ => panic!("expected stdio transport"),
                }
            };
            assert!(pid.is_some());

            state.mcp_disconnect_all().await;
            {
                let pool = state.mcp_sessions.lock().await;
                assert!(pool.is_empty(), "pool drained on disconnect_all");
            }
            // 给 kill 一点时间生效后确认进程不再存活。
            tokio::time::sleep(Duration::from_millis(200)).await;
            if let Some(pid) = pid {
                // kill -0：进程不存在则失败。
                let alive = std::process::Command::new("kill")
                    .args(["-0", &pid.to_string()])
                    .status()
                    .map(|s| s.success())
                    .unwrap_or(false);
                assert!(!alive, "child process should be killed");
            }
            let _ = std::fs::remove_file(&script);
        }

        #[tokio::test]
        async fn config_fingerprint_rebuilds_session() {
            let script = write_fake_server();
            let state = test_app_state();
            let mut server = python_server(&script);

            state
                .mcp_call_tool(&(), &server, "echo", serde_json::json!({ "text": "x" }))
                .await
                .expect("call ok");
            let first_fp = {
                let pool = state.mcp_sessions.lock().await;
                let session = pool.get(&server.id).unwrap().clone();
                drop(pool);
                let guard = session.lock().await;
                guard.config_fingerprint.clone()
            };

            // 改配置（新增 arg）→ fingerprint 变化 → get_or_connect 重建会话。
            server
                .env
                .insert("EXTRA".to_string(), "1".to_string());
            state
                .mcp_call_tool(&(), &server, "echo", serde_json::json!({ "text": "y" }))
                .await
                .expect("call ok after config change");
            let second_fp = {
                let pool = state.mcp_sessions.lock().await;
                let session = pool.get(&server.id).unwrap().clone();
                drop(pool);
                let guard = session.lock().await;
                guard.config_fingerprint.clone()
            };
            assert_ne!(first_fp, second_fp, "config change must rebuild session");

            state.mcp_disconnect_all().await;
            let _ = std::fs::remove_file(&script);
        }
    }
}

