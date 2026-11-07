//! Application MCP connection reuse, OAuth, discovery policy, and offline schemas.

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter};
use tokio::sync::Mutex;

use crate::settings::{ChatMcpServer, Settings};

use super::conn;
use super::types::{McpTool, McpToolCallResult};
use super::McpRuntimeState;
use crate::chat::agent::control::AgentRunHandle;
use kivio_agent_sdk::mcp::McpClient;

const MCP_DISCOVERY_RETRY_BASE: Duration = Duration::from_secs(2);
const MCP_DISCOVERY_RETRY_MAX: Duration = Duration::from_secs(60);

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

/// 状态事件发射器：就是一个可空的 `AppHandle`。
///
/// 这里原本是个 `McpEventSink` trait，两个实现分别是 `AppHandle`（生产）和 `()`（只在测试里），
/// 而 trait 自己就把全部状态暴露成了 `fn app_handle(&self) -> Option<&AppHandle>` —— 也就是说
/// 它从头到尾等价于 `Option<&AppHandle>`，代价是一个泛型参数穿过约 8 个方法。`None` = 不发事件
/// （headless / 单测）。
pub type McpEventSink<'a> = Option<&'a AppHandle>;

fn emit_server_state(sink: McpEventSink<'_>, server: &ChatMcpServer, state: &McpServerState) {
    if let Some(app) = sink {
        let _ = app.emit(
            "mcp-server-state",
            serde_json::json!({
                "serverId": server.id,
                "serverName": server.name,
                "state": state,
            }),
        );
    }
}

/// 仅靠 server_id 发 Disconnected（reload / reap 用，可能拿不到完整 server）。
fn emit_disconnected(sink: McpEventSink<'_>, server_id: &str) {
    if let Some(app) = sink {
        let _ = app.emit(
            "mcp-server-state",
            serde_json::json!({
                "serverId": server_id,
                "state": McpServerState::Disconnected,
            }),
        );
    }
}

pub struct McpSession {
    pub config_fingerprint: String,
    pub state: McpServerState,
    pub last_used: Instant,
    pub handshake_count: u64,
    pub consecutive_connect_failures: u32,
    pub discovery_retry_after: Option<Instant>,
    pub client: Option<Arc<McpClient>>,
    server_config: Option<ChatMcpServer>,
    ping_unsupported: bool,
}

impl McpSession {
    pub(super) fn placeholder(fingerprint: String) -> Self {
        Self {
            config_fingerprint: fingerprint,
            state: McpServerState::Connecting,
            last_used: Instant::now(),
            handshake_count: 0,
            consecutive_connect_failures: 0,
            discovery_retry_after: None,
            client: None,
            server_config: None,
            ping_unsupported: false,
        }
    }

    pub async fn stderr_tail_text(&self) -> String {
        match &self.client {
            Some(client) => client.stderr_tail().await,
            None => String::new(),
        }
    }
}

/// MCP-owned application service. Its dependencies are supplied by the composition root, so MCP
/// lifecycle code cannot reach unrelated `AppState` domains.
pub(crate) struct McpManager<'a> {
    runtime: &'a McpRuntimeState,
    http: &'a reqwest::Client,
    config: McpManagerConfig,
    settings_persistence: &'a dyn McpSettingsPersistence,
}

#[derive(Clone)]
pub(crate) struct McpManagerConfig {
    idle_timeout: Duration,
    tool_timeout: Duration,
    servers: Vec<ChatMcpServer>,
    tinyfish_url: String,
    tinyfish_auth: Option<crate::settings::ConnectorAuth>,
}

impl McpManagerConfig {
    pub(crate) fn from_settings(settings: &Settings) -> Self {
        Self {
            idle_timeout: Duration::from_millis(settings.chat_tools.mcp_idle_timeout_ms),
            tool_timeout: Duration::from_millis(settings.chat_tools.tool_timeout_ms.max(1_000)),
            servers: settings.chat_tools.servers.clone(),
            tinyfish_url: settings.lens.web_search.tinyfish_mcp_url.clone(),
            tinyfish_auth: settings.lens.web_search.tinyfish_mcp_auth.clone(),
        }
    }
}

pub(crate) trait McpSettingsPersistence: Send + Sync {
    fn store_refreshed_server(&self, sink: McpEventSink<'_>, server: &ChatMcpServer);
}

impl<'a> McpManager<'a> {
    pub(crate) fn new(
        runtime: &'a McpRuntimeState,
        http: &'a reqwest::Client,
        config: McpManagerConfig,
        settings_persistence: &'a dyn McpSettingsPersistence,
    ) -> Self {
        Self {
            runtime,
            http,
            config,
            settings_persistence,
        }
    }

    /// 当前生效的空闲超时（来自设置 `mcp_idle_timeout_ms`）。
    pub fn mcp_idle_timeout(&self) -> Duration {
        self.config.idle_timeout
    }

    /// 取该 server 的工具超时（ms）。
    fn mcp_tool_timeout(&self) -> Duration {
        self.config.tool_timeout
    }

    /// 调用方手里的 server 可能是旧快照。用 settings 里同 id / 同 URL 的 OAuth 盖一层。
    fn overlay_oauth_from_settings(&self, server: &ChatMcpServer) -> ChatMcpServer {
        let mut out = server.clone();
        let mut best = out.auth.clone();
        for existing in &self.config.servers {
            let same = existing.id == out.id
                || (mcp_url_matches(&existing.url, &out.url)
                    && existing
                        .auth
                        .as_ref()
                        .is_some_and(|auth| auth.kind == "oauth"));
            if same {
                best = newer_oauth_auth(best, existing.auth.as_ref());
            }
        }
        if mcp_url_matches(self.config.tinyfish_url.trim(), out.url.trim()) {
            best = newer_oauth_auth(best, self.config.tinyfish_auth.as_ref());
        }
        if let Some(auth) = best {
            crate::connectors::oauth::apply_auth_header(&mut out, &auth.access_token);
            out.auth = Some(auth);
        }
        out
    }

    /// 命中已连接会话则克隆 Arc 返回；否则建立连接（握手一次）。
    ///
    /// 关键：算 fingerprint / 命中判断在持外层池锁时完成，确定要新建后插入
    /// Connecting 占位、**立即释放外层锁**，再 spawn + initialize（不跨外层锁 await）。
    pub async fn mcp_get_or_connect(
        &self,
        sink: McpEventSink<'_>,
        server: &ChatMcpServer,
    ) -> Result<Arc<Mutex<McpSession>>, String> {
        self.mcp_get_or_connect_inner(sink, server, false).await
    }

    async fn mcp_get_or_connect_inner(
        &self,
        sink: McpEventSink<'_>,
        server: &ChatMcpServer,
        respect_discovery_backoff: bool,
    ) -> Result<Arc<Mutex<McpSession>>, String> {
        // 连接前钩子：远程 MCP 的 OAuth token 若临近/已过期，先用 refresh_token 刷新，
        // 更新 server 的 Authorization header 与 auth，并持久化新 token。失败则用旧
        // token 继续连接（记录错误，不 panic）。StreamableHttpMcpClient 不变（仍只发 header）。
        let overlaid = self.overlay_oauth_from_settings(server);
        let refreshed = self.refresh_mcp_oauth(sink, &overlaid, false).await;
        let server = refreshed.as_ref().unwrap_or(&overlaid);
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

        let lookup = self
            .runtime
            .get_or_insert_session(&server.id, || {
                Arc::new(Mutex::new(McpSession::placeholder(fingerprint.clone())))
            })
            .await;
        let resolved = if lookup.inserted {
            Resolved::Fresh(lookup.session)
        } else {
            Resolved::Existing(lookup.session)
        };

        match resolved {
            Resolved::Existing(session) => {
                // 锁会话后重判：可能已被先到的调用者握手成功；或配置已变需重建。
                let mut guard = session.lock().await;
                self.runtime
                    .require_current_session(&server.id, &session)
                    .await?;
                if guard.config_fingerprint == fingerprint
                    && matches!(guard.state, McpServerState::Connected)
                {
                    drop(guard);
                    return Ok(session);
                }
                if guard.config_fingerprint != fingerprint {
                    // A schema snapshot belongs to the exact server config that produced it.
                    guard.config_fingerprint = fingerprint.clone();
                    guard.consecutive_connect_failures = 0;
                    guard.discovery_retry_after = None;
                    let old_transport = guard.client.take();
                    guard.state = McpServerState::Connecting;
                    close_transport(old_transport).await;
                } else if respect_discovery_backoff {
                    if let (McpServerState::Error { message }, Some(retry_after)) =
                        (&guard.state, guard.discovery_retry_after)
                    {
                        let now = Instant::now();
                        if retry_after > now {
                            return Err(format!(
                                "{message} (MCP reconnect is cooling down for {} ms)",
                                retry_after.duration_since(now).as_millis()
                            ));
                        }
                    }
                }
                // Connecting/Error sessions retry here; discovery callers honor the cooldown above.
                self.connect_session(sink, server, &mut guard).await?;
                drop(guard);
                Ok(session)
            }
            Resolved::Fresh(session) => {
                emit_server_state(sink, server, &McpServerState::Connecting);
                let mut guard = session.lock().await;
                self.runtime
                    .require_current_session(&server.id, &session)
                    .await?;
                self.connect_session(sink, server, &mut guard).await?;
                drop(guard);
                Ok(session)
            }
        }
    }

    /// OAuth token 刷新。`force=false`：仅在 `needs_refresh`（临近/已过期，或没有
    /// `expires_at`）时刷新。`force=true`：只要 `can_refresh` 就刷新 —— 给握手 /
    /// `tools/call` 收到 401 或 `Auth required` 用。
    ///
    /// 成功则返回更新后的 server（headers + auth），并写回 settings 里所有绑在
    /// 同一 resource URL 上的 OAuth 槽（`chat_tools.servers` + 脱离列表的凭证）。
    pub(crate) async fn refresh_mcp_oauth(
        &self,
        sink: McpEventSink<'_>,
        server: &ChatMcpServer,
        force: bool,
    ) -> Option<ChatMcpServer> {
        let auth = server.auth.as_ref()?;
        let now = now_unix();
        let should_refresh = if force {
            crate::connectors::oauth::can_refresh(auth)
        } else {
            crate::connectors::oauth::needs_refresh(
                auth,
                now,
                crate::connectors::oauth::REFRESH_LEEWAY_SECS,
            )
        };
        if !should_refresh {
            return None;
        }
        let token_endpoint = auth.token_endpoint.clone()?;
        let refresh_token = auth.refresh_token.clone()?;
        let client_id = auth.client_id.clone();

        match crate::connectors::oauth::refresh_access_token(
            self.http,
            &token_endpoint,
            &refresh_token,
            client_id.as_deref(),
            // RFC 8707：刷新也要带 resource，否则新 token 又丢了 audience 绑定。
            Some(server.url.as_str()),
            auth.client_secret.as_deref(),
        )
        .await
        {
            Ok(token) => {
                let mut updated = server.clone();
                let mut new_auth = updated.auth.take().unwrap_or_default();
                crate::connectors::oauth::apply_refreshed_token(&mut new_auth, &token, now);
                crate::connectors::oauth::apply_auth_header(&mut updated, &token.access_token);
                updated.auth = Some(new_auth);
                self.settings_persistence
                    .store_refreshed_server(sink, &updated);
                Some(updated)
            }
            Err(err) => {
                eprintln!(
                    "OAuth token refresh failed for connector {}: {err}; using existing token",
                    server.name
                );
                None
            }
        }
    }

    /// 在持有会话锁的前提下完成一次握手（不持外层池锁）。失败时写 Error 状态并返回错误。
    /// 握手若是 OAuth 401，强制 refresh_token 换新再握一次，避免用户反复点「重新授权」。
    async fn connect_session(
        &self,
        sink: McpEventSink<'_>,
        server: &ChatMcpServer,
        guard: &mut McpSession,
    ) -> Result<(), String> {
        match self.mcp_connect_into(server, guard).await {
            Ok(()) => {
                self.mark_session_connected(sink, server, guard);
                Ok(())
            }
            Err(err) => {
                if conn::is_oauth_error(&err) {
                    if let Some(updated) = self.refresh_mcp_oauth(sink, server, true).await {
                        guard.config_fingerprint = config_fingerprint(&updated);
                        if self.mcp_connect_into(&updated, guard).await.is_ok() {
                            self.mark_session_connected(sink, &updated, guard);
                            return Ok(());
                        }
                    }
                }
                let stderr_tail = guard.stderr_tail_text().await;
                let message = if stderr_tail.trim().is_empty() {
                    err
                } else {
                    format!("{err}\n{stderr_tail}")
                };
                guard.state = McpServerState::Error {
                    message: message.clone(),
                };
                guard.consecutive_connect_failures =
                    guard.consecutive_connect_failures.saturating_add(1);
                guard.discovery_retry_after = Some(
                    Instant::now() + discovery_retry_backoff(guard.consecutive_connect_failures),
                );
                guard.client.take();
                emit_server_state(
                    sink,
                    server,
                    &McpServerState::Error {
                        message: message.clone(),
                    },
                );
                Err(message)
            }
        }
    }

    fn mark_session_connected(
        &self,
        sink: McpEventSink<'_>,
        server: &ChatMcpServer,
        guard: &mut McpSession,
    ) {
        guard.state = McpServerState::Connected;
        guard.handshake_count = guard.handshake_count.saturating_add(1);
        guard.consecutive_connect_failures = 0;
        guard.discovery_retry_after = None;
        guard.last_used = Instant::now();
        guard.server_config = Some(server.clone());
        if let Some(tools) = guard
            .client
            .as_ref()
            .and_then(|client| client.cached_tools())
        {
            self.remember_mcp_tools(server, &tools);
        }
        emit_server_state(sink, server, &McpServerState::Connected);
    }

    async fn mcp_connect_into(
        &self,
        server: &ChatMcpServer,
        session: &mut McpSession,
    ) -> Result<(), String> {
        close_transport(session.client.take()).await;
        session.client = Some(Arc::new(
            McpClient::connect(conn::client_config(server), self.http.clone()).await?,
        ));
        session.ping_unsupported = false;
        Ok(())
    }

    /// 调用某个 MCP server 的工具：get-or-connect → 锁会话判活 → 释放锁再发请求；
    /// 连接确实死了才重连一次重试。
    ///
    /// HTTP 的 404 session 过期由 rmcp 内部单次 best-effort 重握手处理
    /// （`reinit_on_expired_session`，默认开，且只对 `SessionExpired` 生效 ——
    /// 500 不会触发重连），所以这里不再有 stdio / HTTP 两条分支。
    pub async fn mcp_call_tool(
        &self,
        sink: McpEventSink<'_>,
        server: &ChatMcpServer,
        name: &str,
        arguments: Value,
        control: Option<&AgentRunHandle>,
    ) -> Result<McpToolCallResult, String> {
        if control.is_some_and(|control| !control.active()) {
            return Err("MCP tool call cancelled before dispatch".into());
        }
        let server = self.overlay_oauth_from_settings(server);
        let session = self.mcp_get_or_connect(sink, &server).await?;
        let timeout_dur = self.mcp_tool_timeout();

        // 会话锁只保护生命周期状态迁移：拿到 Arc 后必须先释放锁再 await 响应。
        // rmcp 按 JSON-RPC id 路由响应，所以一次丢响应不会阻塞后面对同一个健康
        // 服务器的请求。
        let service = {
            let mut guard = session.lock().await;
            self.runtime
                .require_current_session(&server.id, &session)
                .await?;
            let dead = guard
                .client
                .as_ref()
                .map(|service| service.is_closed())
                .unwrap_or(true);
            if dead {
                close_transport(guard.client.take()).await;
                guard.state = McpServerState::Connecting;
                emit_server_state(sink, &server, &McpServerState::Connecting);
                self.connect_session(sink, &server, &mut guard).await?;
            }
            // 请求开始时也刷一次 last_used，否则空闲回收器会把一个正在跑的长请求
            // 误判成闲置会话。
            guard.last_used = Instant::now();
            guard.client.clone()
        };
        let Some(service) = service else {
            return Err("MCP transport unavailable".to_string());
        };

        let first = service
            .call_tool(
                name,
                arguments.clone(),
                timeout_dur,
                wait_cancelled(control),
            )
            .await;
        let result = match first {
            Ok(value) => Ok(value.into()),
            Err(failure) => {
                // Only a failed send proves the remote tool did not execute.
                let oauth = conn::is_oauth_error(&failure.message);
                if !failure.replay_safe || control.is_some_and(|control| !control.active()) {
                    Err(failure.message)
                } else {
                    let retry_server = if oauth {
                        self.refresh_mcp_oauth(sink, &server, true)
                            .await
                            .unwrap_or_else(|| server.clone())
                    } else {
                        session
                            .lock()
                            .await
                            .server_config
                            .clone()
                            .unwrap_or_else(|| server.clone())
                    };
                    let retry_service = self
                        .retry_client(sink, &retry_server, &session, &service)
                        .await
                        .map_err(|error| {
                            format!("{} (reconnect also failed: {error})", failure.message)
                        })?;
                    retry_service
                        .call_tool(
                            name,
                            arguments,
                            self.mcp_tool_timeout(),
                            wait_cancelled(control),
                        )
                        .await
                        .map(Into::into)
                        .map_err(|failure| failure.message)
                }
            }
        };
        if result.is_ok() {
            session.lock().await.last_used = Instant::now();
        }
        result
    }

    async fn retry_client(
        &self,
        sink: McpEventSink<'_>,
        server: &ChatMcpServer,
        session: &Arc<Mutex<McpSession>>,
        failed_client: &Arc<McpClient>,
    ) -> Result<Arc<McpClient>, String> {
        let mut guard = session.lock().await;
        // Removal revokes reconnect admission even while callers still hold the old Arc.
        self.runtime
            .require_current_session(&server.id, session)
            .await?;
        let must_reconnect = guard
            .client
            .as_ref()
            .map(|current| Arc::ptr_eq(current, failed_client))
            .unwrap_or(true);
        if must_reconnect {
            close_transport(guard.client.take()).await;
            guard.state = McpServerState::Connecting;
            emit_server_state(sink, server, &McpServerState::Connecting);
            self.connect_session(sink, server, &mut guard).await?;
        }
        guard
            .client
            .clone()
            .ok_or_else(|| "MCP transport unavailable".into())
    }

    /// 返回某个 server 的工具列表（持久会话）。供 list_enabled_tool_catalog 复用。
    ///
    /// 只在服务器发过 `tools/list_changed` 时重列。手写版这条只对 stdio 生效，
    /// 现在 HTTP 也一样 —— rmcp 的 `ClientHandler` 两种传输都收得到通知。
    pub async fn mcp_list_tools(
        &self,
        sink: McpEventSink<'_>,
        server: &ChatMcpServer,
    ) -> Result<Vec<McpTool>, String> {
        let session = self.mcp_get_or_connect_inner(sink, server, true).await?;
        let client = session
            .lock()
            .await
            .client
            .clone()
            .ok_or("MCP transport unavailable")?;
        let tools = client.list_tools().await?;
        let mut guard = session.lock().await;
        if guard
            .client
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, &client))
        {
            self.remember_mcp_tools(guard.server_config.as_ref().unwrap_or(server), &tools);
            guard.last_used = Instant::now();
            return Ok(tools);
        }
        Err("MCP connection changed while listing tools".into())
    }

    /// Last successful tool schema, independent from the transport/session lifetime.
    /// A snapshot is reusable only for the exact config fingerprint that produced it.
    pub async fn mcp_cached_tools(&self, server: &ChatMcpServer) -> Option<Vec<McpTool>> {
        self.get_mcp_tool_snapshot(&server.id, &config_fingerprint(server))
    }

    /// UI-only discovery: never connect, refresh tools, extend idle lifetime, or
    /// wait for an in-progress handshake. A disk snapshot cannot prove absence
    /// of a tool after a server upgrade, so only a current live schema is complete.
    pub async fn mcp_display_tools(&self, server: &ChatMcpServer) -> (Vec<McpTool>, bool) {
        let session = self.runtime.session(&server.id).await;
        if let Some(session) = session {
            if let Ok(guard) = session.try_lock() {
                if guard.config_fingerprint == config_fingerprint(server)
                    && guard.state == McpServerState::Connected
                {
                    if let Some(tools) = guard
                        .client
                        .as_ref()
                        .and_then(|client| client.cached_tools())
                    {
                        return (tools, false);
                    }
                }
            }
        }
        (
            self.mcp_cached_tools(server).await.unwrap_or_default(),
            true,
        )
    }

    fn remember_mcp_tools(&self, server: &ChatMcpServer, tools: &[McpTool]) {
        self.set_mcp_tool_snapshot(
            server.id.clone(),
            config_fingerprint(server),
            tools.to_vec(),
        );
    }

    fn get_mcp_tool_snapshot(
        &self,
        server_id: &str,
        config_fingerprint: &str,
    ) -> Option<Vec<McpTool>> {
        self.runtime.tool_snapshot(server_id, config_fingerprint)
    }

    fn set_mcp_tool_snapshot(
        &self,
        server_id: String,
        config_fingerprint: String,
        tools: Vec<McpTool>,
    ) {
        self.runtime
            .set_tool_snapshot(server_id, config_fingerprint, tools);
    }

    /// 无法降级进列表，只能在系统提示词里声明「已配置但连接失败」。
    pub async fn mcp_unreachable_server_ids(&self) -> Vec<String> {
        let candidates = self.runtime.session_entries().await;
        let mut out = Vec::new();
        for (id, session) in candidates {
            let guard = session.lock().await;
            if matches!(guard.state, McpServerState::Error { .. })
                && self
                    .get_mcp_tool_snapshot(&id, &guard.config_fingerprint)
                    .is_none()
            {
                out.push(id);
            }
        }
        out
    }

    /// 读取某个 server 的状态快照（给状态命令）。无会话 ⇒ Disconnected。
    pub async fn mcp_server_state(&self, server_id: &str) -> McpServerStatusSnapshot {
        let session = self.runtime.session(server_id).await;
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
    pub async fn mcp_reload_server(&self, sink: McpEventSink<'_>, server_id: &str) {
        let removed = self.runtime.remove_session(server_id).await;
        if let Some(session) = removed {
            shutdown_session(&session).await;
        }
        emit_disconnected(sink, server_id);
    }

    /// 空闲回收：移除 last_used 超过 idle_timeout 的会话。
    ///
    /// stdio 必须收：Drop 才能杀掉子进程。Streamable HTTP 没有子进程，回收只会让
    /// MCP 页状态点闪成「未连接」，下次握手还可能踩过期 OAuth token。活着的 HTTP
    /// 会话留给 ping 保活、session 过期重握手、主动 reload、退出 disconnect_all。
    /// Error / 占位会话仍回收，避免失败条目永远占着池。
    pub async fn mcp_reap_idle(
        &self,
        idle_timeout: Duration,
    ) -> Vec<(String, Arc<Mutex<McpSession>>)> {
        let now = Instant::now();
        let mut evicted = Vec::new();
        // 先并发拿每个会话的 last_used 需要锁会话；为避免锁内 await，这里改为：
        // 收集所有 (id, Arc)，释放池锁后逐个判断。
        let candidates = self.runtime.session_entries().await;
        let mut expired_ids = Vec::new();
        for (id, session) in &candidates {
            let guard = session.lock().await;
            if now.duration_since(guard.last_used) <= idle_timeout {
                continue;
            }
            let live_http = matches!(guard.state, McpServerState::Connected)
                && guard
                    .client
                    .as_ref()
                    .is_some_and(|client| client.child_pid().is_none());
            if live_http {
                continue;
            }
            expired_ids.push(id.clone());
        }
        if expired_ids.is_empty() {
            return evicted;
        }
        evicted.extend(self.runtime.remove_sessions(&expired_ids).await);
        for (_, session) in &evicted {
            shutdown_session(session).await;
        }
        evicted
    }

    /// HTTP 会话保活：对池里活着的 Streamable HTTP 连接发 MCP `ping`。
    /// 成功续 last_used；对端没有 ping 则记住不再打；其余失败只摘掉连接，
    /// 下次工具调用再握手（定时器里重连会每分钟打一轮 initialize）。
    pub async fn mcp_keepalive_http(&self, sink: McpEventSink<'_>) {
        let candidates = self.runtime.session_entries().await;
        for (id, session) in candidates {
            let (service, skip_ping) = {
                let guard = session.lock().await;
                let live_http = matches!(guard.state, McpServerState::Connected)
                    && guard
                        .client
                        .as_ref()
                        .is_some_and(|client| client.child_pid().is_none());
                if !live_http {
                    continue;
                }
                (guard.client.clone(), guard.ping_unsupported)
            };
            let Some(service) = service else {
                continue;
            };
            if skip_ping {
                continue;
            }
            let result = service.ping(conn::PING_TIMEOUT).await;
            let mut guard = session.lock().await;
            if !guard
                .client
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, &service))
            {
                continue;
            }
            match result {
                Ok(()) => guard.last_used = Instant::now(),
                Err(err) if conn::is_method_not_found(&err) => {
                    guard.ping_unsupported = true;
                    guard.last_used = Instant::now();
                }
                Err(_) => {
                    let client = guard.client.take();
                    guard.state = McpServerState::Disconnected;
                    drop(guard);
                    close_transport(client).await;
                    emit_disconnected(sink, &id);
                }
            }
        }
    }

    /// 排干连接池：每个会话 Drop transport 触发 abort task + kill 子进程。退出钩子用。
    ///
    /// 并发关，等待 SDK 完成请求取消和 transport 清理。多个装死的
    /// server 串起来会延长退出等待。
    pub async fn mcp_disconnect_all(&self) {
        let drained = self.runtime.drain_sessions().await;
        futures::future::join_all(
            drained
                .iter()
                .map(|(_, session)| async move { shutdown_session(session).await }),
        )
        .await;
    }

    /// 断开并移除单个 server 的持久会话（插件关闭 / 卸载时用）。
    pub async fn mcp_disconnect_server(&self, server_id: &str) {
        let session = self.runtime.remove_session(server_id).await;
        if let Some(session) = session {
            shutdown_session(&session).await;
        }
    }
}

impl McpManager<'_> {
    /// Application shutdown fallback only. Normal process cleanup belongs to the SDK client.
    /// 退出兜底：按 pid 杀掉所有 stdio MCP 子进程**及其进程树**，返回杀掉的条数。
    ///
    /// 只在 `mcp_disconnect_all` 超时后用（某个会话锁拿不到，优雅关停这条路走不通）。
    /// 用 `try_lock`：此刻正是「锁拿不到」的场景，再等一次锁等于把退出继续挂住 ——
    /// 拿不到锁的那条会话恰恰是最需要被杀的那条，所以退回读它的 pid 也不行，只能跳过。
    ///
    /// 杀树而不只杀直接子进程：MCP server 自己也可能拉起子进程（`npx` → node），
    /// `kill_on_drop` 够不到孙子。
    pub fn kill_mcp_children_now(&self) -> usize {
        let Some(sessions) = self.runtime.try_session_values() else {
            return 0;
        };
        let mut killed = 0;
        for session in sessions {
            let Ok(guard) = session.try_lock() else {
                continue;
            };
            if let Some(pid) = guard.client.as_ref().and_then(|client| client.child_pid()) {
                crate::native_tools::kill_process_group(pid);
                killed += 1;
            }
        }
        killed
    }
}

/// 把一条已经从池里摘掉的会话关干净：标 Disconnected、摘 transport、关连接。
async fn shutdown_session(session: &Arc<Mutex<McpSession>>) {
    let transport = {
        let mut guard = session.lock().await;
        guard.state = McpServerState::Disconnected;
        guard.client.take()
    };
    close_transport(transport).await;
}

async fn close_transport(client: Option<Arc<McpClient>>) {
    if let Some(client) = client {
        if let Err(error) = client.close().await {
            eprintln!("Failed to close MCP client: {error}");
        }
    }
}

async fn wait_cancelled(control: Option<&AgentRunHandle>) {
    match control {
        Some(control) => control.wait_inactive().await,
        None => std::future::pending().await,
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServerStatusSnapshot {
    pub server_id: String,
    pub state: McpServerState,
    pub handshake_count: u64,
    pub stderr_tail: String,
}

fn discovery_retry_backoff(consecutive_failures: u32) -> Duration {
    let shift = consecutive_failures.saturating_sub(1).min(5);
    let multiplier = 1u32 << shift;
    MCP_DISCOVERY_RETRY_BASE
        .saturating_mul(multiplier)
        .min(MCP_DISCOVERY_RETRY_MAX)
}

/// ChatMcpServer configuration fingerprint; a changed config rebuilds the session.
pub fn config_fingerprint(server: &ChatMcpServer) -> String {
    let serialized = serde_json::to_vec(server)
        .unwrap_or_else(|_| format!("{}:{}", server.id, server.command).into_bytes());
    format!("{:x}", Sha256::digest(serialized))
}

fn mcp_url_matches(left: &str, right: &str) -> bool {
    left.trim().trim_end_matches('/') == right.trim().trim_end_matches('/')
}

/// 把刷新后的 OAuth 写回 settings：`chat_tools.servers`（按 id，以及同 URL 的 oauth
/// 连接器）和任何挂在同一 resource URL 上、存在列表外的凭证槽。
pub(crate) fn apply_refreshed_auth_to_settings(
    settings: &mut crate::settings::Settings,
    server: &ChatMcpServer,
) -> bool {
    let mut changed = false;
    for existing in &mut settings.chat_tools.servers {
        let same_id = existing.id == server.id;
        let same_oauth_url = mcp_url_matches(&existing.url, &server.url)
            && existing
                .auth
                .as_ref()
                .is_some_and(|auth| auth.kind == "oauth");
        if same_id || same_oauth_url {
            existing.auth = server.auth.clone();
            if let Some(value) = server.headers.get("Authorization") {
                existing
                    .headers
                    .insert("Authorization".to_string(), value.clone());
            }
            changed = true;
        }
    }
    if let Some(slot) = settings.detached_oauth_auth_for_url_mut(&server.url) {
        if let Some(auth) = server.auth.clone() {
            *slot = Some(auth);
            changed = true;
        }
    }
    changed
}

fn newer_oauth_auth(
    current: Option<crate::settings::ConnectorAuth>,
    other: Option<&crate::settings::ConnectorAuth>,
) -> Option<crate::settings::ConnectorAuth> {
    match (current, other) {
        (None, other) => other.filter(|auth| auth.kind == "oauth").cloned(),
        (Some(current), None) => Some(current),
        (Some(current), Some(other)) => {
            if other.kind == "oauth"
                && other.expires_at.unwrap_or(0) > current.expires_at.unwrap_or(0)
            {
                Some(other.clone())
            } else {
                Some(current)
            }
        }
    }
}

/// 设置页整份草稿保存时，不要把内存里刚刷新的 OAuth 盖回旧 token。
/// 用户清空该槽（incoming 为 None）仍尊重；导入备份不要走这条。
pub(crate) fn preserve_live_oauth(
    incoming: &mut crate::settings::Settings,
    live: &crate::settings::Settings,
) {
    for live_server in &live.chat_tools.servers {
        let Some(live_auth) = live_server
            .auth
            .as_ref()
            .filter(|auth| auth.kind == "oauth")
        else {
            continue;
        };
        for incoming_server in &mut incoming.chat_tools.servers {
            let same = incoming_server.id == live_server.id
                || (mcp_url_matches(&incoming_server.url, &live_server.url)
                    && incoming_server
                        .auth
                        .as_ref()
                        .is_some_and(|auth| auth.kind == "oauth"));
            if !same {
                continue;
            }
            let Some(chosen) = crate::connectors::oauth::prefer_live_oauth_auth(
                incoming_server.auth.as_ref(),
                Some(live_auth),
            ) else {
                continue;
            };
            if incoming_server
                .auth
                .as_ref()
                .is_some_and(|auth| auth.access_token == chosen.access_token)
            {
                continue;
            }
            crate::connectors::oauth::apply_auth_header(incoming_server, &chosen.access_token);
            incoming_server.auth = Some(chosen);
        }
    }
    let url = incoming.lens.web_search.tinyfish_mcp_url.clone();
    if !mcp_url_matches(url.trim(), live.lens.web_search.tinyfish_mcp_url.trim()) {
        return;
    }
    if let Some(slot) = incoming.detached_oauth_auth_for_url_mut(&url) {
        *slot = crate::connectors::oauth::prefer_live_oauth_auth(
            slot.as_ref(),
            live.lens.web_search.tinyfish_mcp_auth.as_ref(),
        );
    }
}

/// 当前 unix 时间戳（秒），用于 OAuth token 过期判断。
fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn passive_display_requires_a_current_live_schema_and_never_waits_for_handshake() {
        let state = crate::state::test_app_state();
        let server = stdio_server("unused-display-server", &[]);
        let session = Arc::new(Mutex::new(McpSession::placeholder(config_fingerprint(
            &server,
        ))));
        state
            .mcp_test_insert_session(&server.id, session.clone())
            .await;
        assert!(state.mcp_display_tools(&server).await.1);
        let _handshake = session.lock().await;
        let result =
            tokio::time::timeout(Duration::from_millis(100), state.mcp_display_tools(&server))
                .await;
        assert!(
            result
                .expect("passive display must not wait for the session lock")
                .1
        );
    }
    use crate::state::test_app_state;

    #[test]
    fn config_fingerprint_changes_with_config() {
        let mut server = stdio_server("echo", &[]);
        let a = config_fingerprint(&server);
        server.args = vec!["--flag".to_string()];
        let b = config_fingerprint(&server);
        assert_ne!(a, b);
        // Stable for the same config, and safe to persist without raw credentials.
        assert_eq!(b, config_fingerprint(&server));
        server
            .headers
            .insert("Authorization".to_string(), "Bearer top-secret".to_string());
        let credential_fingerprint = config_fingerprint(&server);
        assert_eq!(credential_fingerprint.len(), 64);
        assert!(!credential_fingerprint.contains("top-secret"));
    }

    #[test]
    fn apply_refreshed_auth_writes_detached_oauth_slot_and_matching_server() {
        use crate::settings::{ConnectorAuth, Settings};

        let oauth = ConnectorAuth {
            kind: "oauth".to_string(),
            access_token: "old".to_string(),
            refresh_token: Some("rt".to_string()),
            expires_at: Some(1),
            token_endpoint: Some("https://auth.example/token".to_string()),
            client_id: Some("client".to_string()),
            client_secret: None,
            scopes: Vec::new(),
            account: None,
        };
        let mut settings = Settings::default();
        settings.lens.web_search.tinyfish_mcp_url = "https://agent.tinyfish.ai/mcp".to_string();
        settings.lens.web_search.tinyfish_mcp_auth = Some(oauth.clone());
        let mut connector = ChatMcpServer {
            id: "connector-tinyfish".to_string(),
            name: "TinyFish".to_string(),
            enabled: true,
            transport: "streamable_http".to_string(),
            url: "https://agent.tinyfish.ai/mcp".to_string(),
            ..ChatMcpServer::default()
        };
        connector.auth = Some(oauth);
        connector
            .headers
            .insert("X-Custom".to_string(), "keep".to_string());
        settings.chat_tools.servers.push(connector);

        let mut refreshed = ChatMcpServer {
            id: "tinyfish-mcp".to_string(),
            name: "TinyFish MCP".to_string(),
            enabled: true,
            transport: "streamable_http".to_string(),
            url: "https://agent.tinyfish.ai/mcp".to_string(),
            ..ChatMcpServer::default()
        };
        refreshed
            .headers
            .insert("Authorization".to_string(), "Bearer new".to_string());
        refreshed.auth = Some(ConnectorAuth {
            kind: "oauth".to_string(),
            access_token: "new".to_string(),
            refresh_token: Some("rt".to_string()),
            expires_at: Some(9_999),
            token_endpoint: Some("https://auth.example/token".to_string()),
            client_id: Some("client".to_string()),
            client_secret: None,
            scopes: Vec::new(),
            account: None,
        });

        assert!(apply_refreshed_auth_to_settings(&mut settings, &refreshed));
        assert_eq!(
            settings
                .lens
                .web_search
                .tinyfish_mcp_auth
                .as_ref()
                .unwrap()
                .access_token,
            "new"
        );
        assert_eq!(
            settings.chat_tools.servers[0]
                .auth
                .as_ref()
                .unwrap()
                .access_token,
            "new"
        );
        assert_eq!(
            settings.chat_tools.servers[0]
                .headers
                .get("Authorization")
                .map(String::as_str),
            Some("Bearer new")
        );
        assert_eq!(
            settings.chat_tools.servers[0]
                .headers
                .get("X-Custom")
                .map(String::as_str),
            Some("keep")
        );
    }

    #[test]
    fn preserve_live_oauth_keeps_newer_token_on_stale_draft() {
        use crate::settings::{ConnectorAuth, Settings};

        let mut live = Settings::default();
        let mut incoming = Settings::default();
        let mut server = ChatMcpServer {
            id: "s1".to_string(),
            name: "S".to_string(),
            enabled: true,
            transport: "streamable_http".to_string(),
            url: "https://mcp.example/mcp".to_string(),
            ..ChatMcpServer::default()
        };
        server.auth = Some(ConnectorAuth {
            kind: "oauth".to_string(),
            access_token: "old".to_string(),
            refresh_token: Some("rt".to_string()),
            expires_at: Some(10),
            token_endpoint: Some("https://auth.example/token".to_string()),
            client_id: Some("c".to_string()),
            client_secret: None,
            scopes: Vec::new(),
            account: None,
        });
        incoming.chat_tools.servers.push(server.clone());
        server.auth.as_mut().unwrap().access_token = "new".to_string();
        server.auth.as_mut().unwrap().expires_at = Some(9_999);
        crate::connectors::oauth::apply_auth_header(&mut server, "new");
        live.chat_tools.servers.push(server);

        preserve_live_oauth(&mut incoming, &live);
        assert_eq!(
            incoming.chat_tools.servers[0]
                .auth
                .as_ref()
                .unwrap()
                .access_token,
            "new"
        );
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
            connector_id: None,
            auth: None,
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
            connector_id: None,
            auth: None,
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
    async fn disconnected_session_cannot_reconnect_from_a_retained_client() {
        let state = test_app_state();
        let directory = tempfile::tempdir().unwrap();
        let runtime = McpRuntimeState::load(directory.path());
        let manager = McpManager::new(
            &runtime,
            &state.http,
            McpManagerConfig::from_settings(&Settings::default()),
            &state,
        );
        let server = http_server(spawn_test_http_mcp_server(200).await);
        let session = manager.mcp_get_or_connect(None, &server).await.unwrap();
        let retained = session.lock().await.client.clone().unwrap();

        retained.close().await.unwrap();
        let replacement = manager
            .retry_client(None, &server, &session, &retained)
            .await
            .unwrap();
        assert!(!Arc::ptr_eq(&retained, &replacement));
        assert_eq!(
            manager.mcp_server_state(&server.id).await.handshake_count,
            2
        );

        manager.mcp_reload_server(None, &server.id).await;
        let failure = replacement
            .call_tool(
                "echo",
                serde_json::json!({}),
                Duration::from_secs(1),
                std::future::pending(),
            )
            .await
            .expect_err("retained client was closed by reload");
        assert!(failure.replay_safe, "the remote call was never sent");
        let error = manager
            .retry_client(None, &server, &session, &replacement)
            .await
            .expect_err("removed session must not reconnect");
        assert!(error.contains("disconnected"), "{error}");
        assert!(runtime.sessions_empty().await);
        assert!(session.lock().await.client.is_none());
        assert_eq!(session.lock().await.handshake_count, 2);

        let current = manager.mcp_get_or_connect(None, &server).await.unwrap();
        assert!(!Arc::ptr_eq(&session, &current));
        assert_eq!(runtime.session_count().await, 1);
        assert!(manager
            .retry_client(None, &server, &session, &replacement)
            .await
            .is_err());
        manager.mcp_disconnect_all().await;
    }

    #[tokio::test]
    async fn http_reconnect_only_on_404() {
        // 404 → 清 session 重 initialize 重试 → 成功。
        let url = spawn_test_http_mcp_server(404).await;
        let state = test_app_state();
        let server = http_server(url);
        let result = state
            .mcp_call_tool(None, &server, "echo", serde_json::json!({}), None)
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
            .mcp_call_tool(None, &server, "echo", serde_json::json!({}), None)
            .await
            .expect_err("500 should surface as error");
        assert!(err.contains("500"), "got: {err}");
        state.mcp_disconnect_all().await;
    }

    #[tokio::test]
    async fn connect_once_lists_tools_without_touching_the_pool() {
        // 设置页的「测试连接」测的是还没保存的草稿配置，绝不能进连接池 ——
        // 否则一个用户还在编辑的半成品配置会被当成常驻服务器缓存下来。
        let url = spawn_test_http_mcp_server(200).await;
        let state = test_app_state();
        let server = http_server(url);

        let tools = conn::list_tools_once(&server, &state.http)
            .await
            .expect("one-shot list should work");
        assert_eq!(
            tools.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
            vec!["echo"]
        );
        assert!(
            state.mcp_test_sessions_empty().await,
            "一次性连接不得进连接池"
        );
    }

    #[tokio::test]
    async fn call_tool_once_works_and_stays_out_of_the_pool() {
        // web_search 的 exa server 是临时合成的（api key 在 URL 里），同样不该进池。
        let url = spawn_test_http_mcp_server(200).await;
        let state = test_app_state();
        let server = http_server(url);

        let raw = conn::call_tool_once(
            &server,
            &state.http,
            "echo",
            serde_json::json!({}),
            Duration::from_secs(5),
        )
        .await
        .expect("one-shot call should work");
        let parsed = raw;
        assert_eq!(parsed.content, "echo: ok");
        assert!(state.mcp_test_sessions_empty().await);
    }

    #[tokio::test]
    async fn idle_reap_keeps_live_http_sessions() {
        let url = spawn_test_http_mcp_server(200).await;
        let state = test_app_state();
        let server = http_server(url);
        state
            .mcp_call_tool(None, &server, "echo", serde_json::json!({}), None)
            .await
            .expect("call ok");
        tokio::time::sleep(Duration::from_millis(20)).await;
        let evicted = state.mcp_reap_idle(Duration::from_millis(1)).await;
        assert!(
            evicted.is_empty(),
            "live HTTP MCP sessions must not idle-reap (UI would flash Disconnected)"
        );
        assert!(
            state.mcp_test_has_session(&server.id).await,
            "HTTP session should stay pooled"
        );
        state.mcp_disconnect_all().await;
    }

    #[tokio::test]
    async fn http_keepalive_ping_reuses_session() {
        let url = spawn_test_http_mcp_server(200).await;
        let state = test_app_state();
        let server = http_server(url);
        state
            .mcp_call_tool(None, &server, "echo", serde_json::json!({}), None)
            .await
            .expect("call ok");
        let handshake_before = {
            let session = state.mcp_test_session(&server.id).await.expect("pooled");
            let count = session.lock().await.handshake_count;
            count
        };
        state.mcp_keepalive_http(None).await;
        {
            let session = state.mcp_test_session(&server.id).await.expect("pooled");
            let guard = session.lock().await;
            assert_eq!(
                guard.handshake_count, handshake_before,
                "live HTTP ping must not re-handshake"
            );
            assert!(matches!(guard.state, McpServerState::Connected));
            assert!(guard.client.is_some());
        }
        state.mcp_disconnect_all().await;
    }

    /// fake HTTP MCP server：始终 200，计数收到的 initialize 次数（用于断言会话复用）。
    /// 返回 (url, initialize_count)。
    async fn spawn_counting_http_mcp_server() -> (
        String,
        std::sync::Arc<std::sync::atomic::AtomicU64>,
        std::sync::Arc<std::sync::atomic::AtomicU64>,
        std::sync::Arc<std::sync::atomic::AtomicU64>,
    ) {
        use std::sync::atomic::{AtomicU64, Ordering};
        use std::sync::Arc;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        let init_count = Arc::new(AtomicU64::new(0));
        let negotiated_header_count = Arc::new(AtomicU64::new(0));
        let delete_count = Arc::new(AtomicU64::new(0));
        let init_count_server = init_count.clone();
        let negotiated_header_count_server = negotiated_header_count.clone();
        let delete_count_server = delete_count.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                let init_count = init_count_server.clone();
                let negotiated_header_count = negotiated_header_count_server.clone();
                let delete_count = delete_count_server.clone();
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
                                line.split_once(':').and_then(|(name, value)| {
                                    name.eq_ignore_ascii_case("content-length")
                                        .then(|| value.trim().parse::<usize>().ok())
                                        .flatten()
                                })
                            })
                            .unwrap_or(0);
                        if read < header_end + 4 + content_length {
                            continue;
                        }
                        let http_method = request
                            .lines()
                            .next()
                            .and_then(|line| line.split_whitespace().next())
                            .unwrap_or_default();
                        let header_value = |name: &str| {
                            request.lines().find_map(|line| {
                                line.split_once(':').and_then(|(header_name, value)| {
                                    header_name
                                        .eq_ignore_ascii_case(name)
                                        .then(|| value.trim().to_string())
                                })
                            })
                        };
                        let negotiated_headers_ok = header_value("mcp-protocol-version").as_deref()
                            == Some("2025-03-26")
                            && header_value("mcp-session-id").as_deref() == Some("sess-1");

                        if http_method == "DELETE" {
                            if negotiated_headers_ok {
                                delete_count.fetch_add(1, Ordering::SeqCst);
                            }
                            let raw =
                                "HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                            let _ = stream.write_all(raw.as_bytes()).await;
                            let _ = stream.shutdown().await;
                            return;
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
                        } else if negotiated_headers_ok {
                            negotiated_header_count.fetch_add(1, Ordering::SeqCst);
                        }

                        let response = match method {
                            "initialize" => serde_json::json!({
                                "jsonrpc":"2.0","id":id,
                                "result":{"protocolVersion":"2025-03-26","capabilities":{"tools":{}},"serverInfo":{"name":"fake","version":"1.0.0"}}
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
        (
            format!("http://{addr}/mcp"),
            init_count,
            negotiated_header_count,
            delete_count,
        )
    }

    #[tokio::test]
    async fn http_reuses_negotiated_version_and_deletes_session() {
        use std::sync::atomic::Ordering;
        let (url, init_count, negotiated_header_count, delete_count) =
            spawn_counting_http_mcp_server().await;
        let state = test_app_state();
        let server = http_server(url);

        let first = state
            .mcp_call_tool(None, &server, "echo", serde_json::json!({}), None)
            .await
            .expect("first call ok");
        assert_eq!(first.content, "echo: ok");
        let second = state
            .mcp_call_tool(None, &server, "echo", serde_json::json!({}), None)
            .await
            .expect("second call ok");
        assert_eq!(second.content, "echo: ok");

        assert_eq!(
            init_count.load(Ordering::SeqCst),
            1,
            "two successful HTTP calls must share a single initialize"
        );
        assert!(
            negotiated_header_count.load(Ordering::SeqCst) >= 3,
            "initialized and later requests must use the negotiated protocol version and session id"
        );

        state.mcp_disconnect_all().await;
        assert_eq!(
            delete_count.load(Ordering::SeqCst),
            1,
            "disconnect must DELETE the negotiated HTTP session"
        );
    }

    /// stdio 真机用例：起一个 python fake server。
    ///
    /// `#[cfg(unix)]` 是刻意的（脚本 + `kill -0` 都吃 unix），**代价是 Windows 上
    /// `cargo test` 完全不编译这个模块** —— 本地全绿不代表这里绿。CI 在 macOS runner 上
    /// 跑 `cargo test` 就是为了守住这块；改动本模块引用的类型时别只看本地结果。
    #[cfg(unix)]
    mod stdio {
        use super::*;
        use std::io::Write;

        /// 进程还活着吗。`kill -0`：进程不存在则失败。
        ///
        /// 不能用 `service.is_closed()` 代替 —— rmcp 的服务循环退出时不 cancel token，
        /// 那个方法对池里的连接永远返回 false。
        fn process_alive(pid: u32) -> bool {
            std::process::Command::new("kill")
                .args(["-0", &pid.to_string()])
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
        }

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
        if text == "oauth-error":
            resp = {"jsonrpc":"2.0","id":mid,"error":{"code":-32603,"message":"Auth required after executing tool"}}
            sys.stdout.write(json.dumps(resp)+"\n")
            sys.stdout.flush()
            continue
        if text == "hang":
            # Simulate a buggy MCP server that loses one response but remains healthy.
            continue
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
            file.write_all(script.as_bytes())
                .expect("write fake server");
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
                    .mcp_call_tool(
                        None,
                        &server,
                        "echo",
                        serde_json::json!({ "text": i }),
                        None,
                    )
                    .await
                    .expect("call should succeed");
                assert_eq!(result.content, format!("echo: {i}"));
            }

            let handshake_count = {
                let session = state
                    .mcp_test_session(&server.id)
                    .await
                    .expect("session present");
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
            state.update_settings_for_test(|settings| settings.chat_tools.tool_timeout_ms = 1_000);

            let mut marker = std::env::temp_dir();
            marker.push(format!(
                "kivio-fake-mcp-marker-{}.txt",
                uuid::Uuid::new_v4()
            ));

            let mut server = python_server(&script);
            server
                .env
                .insert("KIVIO_DELAY_CALL_MS".to_string(), "2500".to_string());
            server.env.insert(
                "KIVIO_CALL_MARKER".to_string(),
                marker.to_string_lossy().into_owned(),
            );

            let err = state
                .mcp_call_tool(
                    None,
                    &server,
                    "echo",
                    serde_json::json!({ "text": "slow" }),
                    None,
                )
                .await
                .expect_err("slow healthy tool should surface a timeout error");
            assert!(
                err.contains("timed out"),
                "expected a timeout error, got: {err}"
            );

            // 握手仍为 1（未重连），子进程仍存活。
            let (handshake_count, pid) = {
                let session = state
                    .mcp_test_session(&server.id)
                    .await
                    .expect("session present");
                let guard = session.lock().await;
                // Verify the actual child process, not only the local client state.
                let pid = guard.client.as_ref().and_then(|client| client.child_pid());
                assert!(
                    pid.map(process_alive).unwrap_or(false),
                    "healthy child must not be killed by a timeout"
                );
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
            let h1 =
                tokio::spawn(async move { s1.mcp_get_or_connect(None, &srv1).await.map(|_| ()) });
            let h2 =
                tokio::spawn(async move { s2.mcp_get_or_connect(None, &srv2).await.map(|_| ()) });
            let (r1, r2) = tokio::join!(h1, h2);
            r1.unwrap().expect("connect one ok");
            r2.unwrap().expect("connect two ok");

            let (entries, handshake_count) = {
                let entries = state.mcp_test_session_count().await;
                let session = state
                    .mcp_test_session(&server.id)
                    .await
                    .expect("session present");
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
                .mcp_call_tool(
                    None,
                    &server,
                    "echo",
                    serde_json::json!({ "text": "a" }),
                    None,
                )
                .await
                .expect("first call ok");
            assert_eq!(first.content, "echo: a");

            // 给子进程一点时间真正退出。
            tokio::time::sleep(Duration::from_millis(200)).await;

            let second = state
                .mcp_call_tool(
                    None,
                    &server,
                    "echo",
                    serde_json::json!({ "text": "b" }),
                    None,
                )
                .await
                .expect("second call should transparently reconnect");
            assert_eq!(second.content, "echo: b");

            let handshake_count = {
                let session = state
                    .mcp_test_session(&server.id)
                    .await
                    .expect("session present");
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
        async fn lost_response_does_not_head_of_line_block_later_request() {
            // The fake server deliberately drops the response for "hang" but keeps reading stdin.
            // A later request on the same stdio session must still reach the server and complete.
            let script = write_fake_server();
            let state = std::sync::Arc::new(test_app_state());
            state.update_settings_for_test(|settings| settings.chat_tools.tool_timeout_ms = 1_000);
            let server = python_server(&script);

            state
                .mcp_call_tool(
                    None,
                    &server,
                    "echo",
                    serde_json::json!({ "text": "warm" }),
                    None,
                )
                .await
                .expect("warmup ok");

            let first_state = state.clone();
            let first_server = server.clone();
            let first = tokio::spawn(async move {
                first_state
                    .mcp_call_tool(
                        None,
                        &first_server,
                        "echo",
                        serde_json::json!({ "text": "hang" }),
                        None,
                    )
                    .await
            });

            tokio::time::sleep(Duration::from_millis(100)).await;

            let second_state = state.clone();
            let second_server = server.clone();
            let second = tokio::time::timeout(Duration::from_millis(500), async move {
                second_state
                    .mcp_call_tool(
                        None,
                        &second_server,
                        "echo",
                        serde_json::json!({ "text": "two" }),
                        None,
                    )
                    .await
            })
            .await
            .expect("second request must not wait for the first request timeout")
            .expect("second request should succeed");
            assert_eq!(second.content, "echo: two");

            let first_err = first
                .await
                .expect("first task join")
                .expect_err("the deliberately lost response should time out");
            assert!(first_err.contains("outcome is unknown"), "{first_err}");
            assert!(first_err.contains("was not retried"), "{first_err}");

            state.mcp_disconnect_all().await;
            let _ = std::fs::remove_file(&script);
        }

        #[tokio::test]
        async fn idle_reap_evicts_and_reconnects() {
            let script = write_fake_server();
            let state = test_app_state();
            let server = python_server(&script);

            state
                .mcp_call_tool(
                    None,
                    &server,
                    "echo",
                    serde_json::json!({ "text": "x" }),
                    None,
                )
                .await
                .expect("call ok");
            assert!(state.mcp_test_has_session(&server.id).await);

            // 注入极小空闲超时 → 立即过期回收。
            tokio::time::sleep(Duration::from_millis(20)).await;
            let evicted = state.mcp_reap_idle(Duration::from_millis(1)).await;
            assert_eq!(evicted.len(), 1);
            assert!(
                !state.mcp_test_has_session(&server.id).await,
                "session should be reaped"
            );

            // 回收后下次调用透明重连。
            let again = state
                .mcp_call_tool(
                    None,
                    &server,
                    "echo",
                    serde_json::json!({ "text": "y" }),
                    None,
                )
                .await
                .expect("reconnect after reap ok");
            assert_eq!(again.content, "echo: y");

            // 回收后的重连必须是一次全新握手。回收会把旧会话整个移出连接池
            // （上面已断言 !contains_key），下次调用新建一个全新会话并握手一次 ⇒
            // 新会话 handshake_count == 1。这证明重连走了全新握手而非复用旧连接。
            let handshake_count = {
                let session = state
                    .mcp_test_session(&server.id)
                    .await
                    .expect("session present");
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
                .mcp_call_tool(
                    None,
                    &server,
                    "echo",
                    serde_json::json!({ "text": "x" }),
                    None,
                )
                .await
                .expect("call ok");
            // 记录子进程 pid。
            let pid = {
                let session = state.mcp_test_session(&server.id).await.unwrap();
                let guard = session.lock().await;
                assert!(guard.client.is_some(), "expected a live stdio transport");
                guard.client.as_ref().and_then(|client| client.child_pid())
            };
            assert!(pid.is_some());

            state.mcp_disconnect_all().await;
            assert!(
                state.mcp_test_sessions_empty().await,
                "pool drained on disconnect_all"
            );
            // 给 kill 一点时间生效后确认进程不再存活。
            tokio::time::sleep(Duration::from_millis(200)).await;
            if let Some(pid) = pid {
                assert!(!process_alive(pid), "child process should be killed");
            }
            let _ = std::fs::remove_file(&script);
        }

        #[tokio::test]
        async fn config_fingerprint_rebuilds_session() {
            let script = write_fake_server();
            let state = test_app_state();
            let mut server = python_server(&script);

            state
                .mcp_call_tool(
                    None,
                    &server,
                    "echo",
                    serde_json::json!({ "text": "x" }),
                    None,
                )
                .await
                .expect("call ok");
            let first_fp = {
                let session = state.mcp_test_session(&server.id).await.unwrap();
                let guard = session.lock().await;
                guard.config_fingerprint.clone()
            };

            // 改配置（新增 arg）→ fingerprint 变化 → get_or_connect 重建会话。
            server.env.insert("EXTRA".to_string(), "1".to_string());
            state
                .mcp_call_tool(
                    None,
                    &server,
                    "echo",
                    serde_json::json!({ "text": "y" }),
                    None,
                )
                .await
                .expect("call ok after config change");
            let second_fp = {
                let session = state.mcp_test_session(&server.id).await.unwrap();
                let guard = session.lock().await;
                guard.config_fingerprint.clone()
            };
            assert_ne!(first_fp, second_fp, "config change must rebuild session");

            state.mcp_disconnect_all().await;
            let _ = std::fs::remove_file(&script);
        }
    }

    /// 跨平台 fake MCP stdio 测试：验证复用会话时也会读取最新的 tool_timeout_ms。
    mod stdio_cross_platform {
        use super::*;
        use std::io::Write;

        fn python_command() -> &'static str {
            if cfg!(windows) {
                "python"
            } else {
                "python3"
            }
        }

        fn write_fake_server() -> std::path::PathBuf {
            let script = r#"#!/usr/bin/env python3
import sys, json, os, time
delay_ms = int(os.environ.get("KIVIO_DELAY_CALL_MS", "0"))
init_delay_ms = int(os.environ.get("KIVIO_DELAY_INIT_MS", "0"))
marker = os.environ.get("KIVIO_CALL_MARKER", "")
changed = False
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
        if method == "notifications/cancelled" and marker:
            with open(marker, "a") as f:
                f.write("cancel:"+str(msg.get("params", {}).get("requestId"))+"\n")
        continue
    if method == "initialize":
        if init_delay_ms:
            time.sleep(init_delay_ms / 1000.0)
        resp = {"jsonrpc":"2.0","id":mid,"result":{"protocolVersion":"2025-06-18","capabilities":{"tools":{}},"serverInfo":{"name":"fake","version":"1.0.0"}}}
    elif method == "tools/list":
        cursor = msg.get("params", {}).get("cursor")
        if cursor is None:
            resp = {"jsonrpc":"2.0","id":mid,"result":{"tools":[{"name":"echo","description":"Echo","inputSchema":{"type":"object","properties":{"text":{"type":"string"}}}}],"nextCursor":"page-2"}}
        else:
            page = [{"name":"second","description":"Second","inputSchema":{"type":"object"}}]
            if changed:
                page.append({"name":"dynamic","description":"Dynamic","inputSchema":{"type":"object"}})
            resp = {"jsonrpc":"2.0","id":mid,"result":{"tools":page}}
    elif method == "tools/call":
        text = ""
        try:
            text = msg["params"]["arguments"].get("text","")
        except Exception:
            text = ""
        if marker:
            with open(marker, "a") as f:
                f.write(str(text)+"\n")
        if text == "oauth-error":
            resp = {"jsonrpc":"2.0","id":mid,"error":{"code":-32603,"message":"Auth required after executing tool"}}
            sys.stdout.write(json.dumps(resp)+"\n")
            sys.stdout.flush()
            continue
        if text == "hang":
            # Simulate a buggy MCP server that loses one response but remains healthy.
            continue
        if text == "ping":
            sys.stdout.write(json.dumps({"jsonrpc":"2.0","id":mid,"method":"ping"})+"\n")
            sys.stdout.flush()
            ping_response = json.loads(sys.stdin.readline())
            if marker:
                with open(marker, "a") as f:
                    f.write("ping-result:"+str(ping_response.get("result"))+"\n")
            sys.stdout.write(json.dumps({"jsonrpc":"2.0","id":"unknown-"+str(mid),"method":"server/unknown"})+"\n")
            sys.stdout.flush()
            unknown_response = json.loads(sys.stdin.readline())
            if marker:
                with open(marker, "a") as f:
                    f.write("unknown-code:"+str(unknown_response.get("error", {}).get("code"))+"\n")
            resp = {"jsonrpc":"2.0","id":str(mid),"result":{"content":[{"type":"text","text":"echo: ping"}]}}
            sys.stdout.write(json.dumps(resp)+"\n")
            sys.stdout.flush()
            continue
        if text == "change":
            changed = True
            sys.stdout.write(json.dumps({"jsonrpc":"2.0","method":"notifications/tools/list_changed"})+"\n")
            sys.stdout.flush()
        if delay_ms:
            time.sleep(delay_ms / 1000.0)
        resp = {"jsonrpc":"2.0","id":mid,"result":{"content":[{"type":"text","text":"echo: "+str(text)}]}}
        sys.stdout.write(json.dumps(resp)+"\n")
        sys.stdout.flush()
        continue
    else:
        resp = {"jsonrpc":"2.0","id":mid,"result":{}}
    sys.stdout.write(json.dumps(resp)+"\n")
    sys.stdout.flush()
"#;
            let mut path = std::env::temp_dir();
            path.push(format!("kivio-fake-mcp-xplat-{}.py", uuid::Uuid::new_v4()));
            let mut file = std::fs::File::create(&path).expect("create fake server");
            file.write_all(script.as_bytes())
                .expect("write fake server");
            path
        }

        fn python_server(script: &std::path::Path) -> ChatMcpServer {
            stdio_server(python_command(), &["-u", script.to_str().unwrap()])
        }

        #[tokio::test]
        async fn cold_catalog_keeps_tools_from_a_server_starting_after_three_seconds() {
            let script = write_fake_server();
            let state = test_app_state();
            let mut server = python_server(&script);
            server
                .env
                .insert("KIVIO_DELAY_INIT_MS".into(), "3200".into());
            let mut settings = crate::settings::Settings::default();
            settings.chat_tools.enabled = true;
            settings.chat_tools.servers = vec![server];
            let (tools, unavailable) =
                crate::mcp::registry::collect_enabled_mcp_tool_defs(&state, None, &settings, None)
                    .await;
            state.mcp_disconnect_all().await;
            let _ = std::fs::remove_file(script);
            assert!(
                unavailable.is_empty(),
                "a healthy cold server must be available on the first turn"
            );
            assert!(tools.iter().any(|tool| tool.name == "echo"));
        }

        /// 一个**只实现 2026-07-28** 的假服务器：对 `initialize` 回 `-32601`
        /// （规范把这个方法删掉了），只认 `server/discover`。
        ///
        /// 现实中这种服务器还不存在，所以这条路只能这么验 —— 而它正是本次改动要防的那件事：
        /// 不做的话，将来第一台这样的服务器出现时 Kivio 是**连不上**，不是降级。
        fn write_discover_only_server() -> std::path::PathBuf {
            let script = r#"#!/usr/bin/env python3
import sys, json
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
        continue
    if method == "initialize":
        # 规范 2026-07-28 删掉了 initialize：按 JSON-RPC 就该回「没这个方法」。
        resp = {"jsonrpc":"2.0","id":mid,"error":{"code":-32601,"message":"Method not found"}}
    elif method == "server/discover":
        resp = {"jsonrpc":"2.0","id":mid,"result":{"resultType":"complete","supportedVersions":["2026-07-28"],"capabilities":{"tools":{}},"ttlMs":0,"cacheScope":"public"}}
    elif method == "tools/list":
        resp = {"jsonrpc":"2.0","id":mid,"result":{"tools":[{"name":"echo","description":"Echo","inputSchema":{"type":"object"}}]}}
    elif method == "tools/call":
        resp = {"jsonrpc":"2.0","id":mid,"result":{"content":[{"type":"text","text":"echo: new-protocol"}]}}
    else:
        resp = {"jsonrpc":"2.0","id":mid,"error":{"code":-32601,"message":"Method not found"}}
    sys.stdout.write(json.dumps(resp)+"\n")
    sys.stdout.flush()
"#;
            let mut path = std::env::temp_dir();
            path.push(format!(
                "kivio-fake-mcp-discover-{}.py",
                uuid::Uuid::new_v4()
            ));
            let mut file = std::fs::File::create(&path).expect("create discover-only server");
            file.write_all(script.as_bytes())
                .expect("write discover-only server");
            path
        }

        /// 只支持新协议的服务器必须能连上、能列工具、能调工具。
        ///
        /// 判据是 `-32601`：`McpClient::connect` 先发 `initialize`（现存服务器全走这条，保持零变化），
        /// 被以「没这个方法」拒掉之后才换 `server/discover` 重试。
        #[tokio::test]
        async fn discover_only_server_connects_after_initialize_is_rejected() {
            let script = write_discover_only_server();
            let state = test_app_state();
            let server = python_server(&script);

            let tools = state
                .mcp_list_tools(None, &server)
                .await
                .expect("只支持 2026-07-28 的服务器必须能连上");
            assert!(
                tools.iter().any(|tool| tool.name == "echo"),
                "工具没列出来: {tools:?}"
            );

            let result = state
                .mcp_call_tool(None, &server, "echo", serde_json::json!({}), None)
                .await
                .expect("新协议下的 tools/call 必须能用");
            assert!(result.content.contains("new-protocol"), "{result:?}");

            state.mcp_disconnect_all().await;
            let _ = std::fs::remove_file(&script);
        }

        /// 反面：普通失败（服务器起不来）**不该**触发第二次握手 —— 那会白起一个子进程。
        /// 判据只认 `-32601`，由 SDK 的协议协商测试钉住；
        /// 这条从 app 层确认它确实只握手一次。
        #[tokio::test]
        async fn a_server_that_cannot_start_is_not_retried_with_the_new_protocol() {
            let state = test_app_state();
            let server = stdio_server("kivio-definitely-not-a-real-binary", &[]);
            let err = state
                .mcp_list_tools(None, &server)
                .await
                .expect_err("起不来的服务器必须失败");
            // 报的是启动失败，而不是 discover 那一路的错误。
            assert!(err.contains("Failed to start MCP server"), "{err}");
        }

        #[tokio::test]
        async fn lost_response_does_not_block_later_request() {
            let script = write_fake_server();
            let state = std::sync::Arc::new(test_app_state());
            state.update_settings_for_test(|settings| settings.chat_tools.tool_timeout_ms = 1_000);
            let mut marker = std::env::temp_dir();
            marker.push(format!("kivio-fake-mcp-hol-{}.txt", uuid::Uuid::new_v4()));
            let mut server = python_server(&script);
            server.env.insert(
                "KIVIO_CALL_MARKER".to_string(),
                marker.to_string_lossy().into_owned(),
            );

            state
                .mcp_call_tool(
                    None,
                    &server,
                    "echo",
                    serde_json::json!({ "text": "warm" }),
                    None,
                )
                .await
                .expect("warmup ok");

            let first_state = state.clone();
            let first_server = server.clone();
            let first = tokio::spawn(async move {
                first_state
                    .mcp_call_tool(
                        None,
                        &first_server,
                        "echo",
                        serde_json::json!({ "text": "hang" }),
                        None,
                    )
                    .await
            });
            tokio::time::sleep(Duration::from_millis(100)).await;

            let second = tokio::time::timeout(
                Duration::from_millis(500),
                state.mcp_call_tool(
                    None,
                    &server,
                    "echo",
                    serde_json::json!({ "text": "two" }),
                    None,
                ),
            )
            .await
            .expect("later request must bypass the lost response")
            .expect("later request should succeed");
            assert_eq!(second.content, "echo: two");

            let first_err = first
                .await
                .expect("first task join")
                .expect_err("lost response should time out");
            assert!(first_err.contains("outcome is unknown"), "{first_err}");
            assert!(first_err.contains("was not retried"), "{first_err}");

            // Writing the notification only guarantees that it reached the child stdin;
            // allow the fake server a short scheduling window to consume it and persist
            // the marker before asserting the observable side effect.
            let mut calls = String::new();
            for _ in 0..50 {
                calls = std::fs::read_to_string(&marker).unwrap_or_default();
                if calls.lines().any(|line| line.starts_with("cancel:")) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            assert_eq!(calls.lines().filter(|line| *line == "hang").count(), 1);
            assert_eq!(calls.lines().filter(|line| *line == "two").count(), 1);
            assert!(
                calls.lines().any(|line| line.starts_with("cancel:")),
                "timed-out request should emit notifications/cancelled: {calls}"
            );

            state.mcp_disconnect_all().await;
            let _ = std::fs::remove_file(&script);
            let _ = std::fs::remove_file(&marker);
        }

        #[tokio::test]
        async fn server_ping_is_answered_without_consuming_the_real_response() {
            let script = write_fake_server();
            let state = test_app_state();
            let mut marker = std::env::temp_dir();
            marker.push(format!("kivio-fake-mcp-ping-{}.txt", uuid::Uuid::new_v4()));
            let mut server = python_server(&script);
            server.env.insert(
                "KIVIO_CALL_MARKER".to_string(),
                marker.to_string_lossy().into_owned(),
            );

            let result = state
                .mcp_call_tool(
                    None,
                    &server,
                    "echo",
                    serde_json::json!({ "text": "ping" }),
                    None,
                )
                .await
                .expect("server ping should be answered and string response id accepted");
            assert_eq!(result.content, "echo: ping");
            let calls = std::fs::read_to_string(&marker).unwrap_or_default();
            assert!(calls.contains("ping-result:{}"), "{calls}");
            assert!(calls.contains("unknown-code:-32601"), "{calls}");

            state.mcp_disconnect_all().await;
            let _ = std::fs::remove_file(&script);
            let _ = std::fs::remove_file(&marker);
        }

        #[tokio::test]
        async fn tools_list_paginates_and_refreshes_after_list_changed() {
            let script = write_fake_server();
            let state = test_app_state();
            let server = python_server(&script);

            let first = state
                .mcp_list_tools(None, &server)
                .await
                .expect("initial paginated tools/list");
            assert_eq!(
                first
                    .iter()
                    .map(|tool| tool.name.as_str())
                    .collect::<Vec<_>>(),
                vec!["echo", "second"]
            );

            state
                .mcp_call_tool(
                    None,
                    &server,
                    "echo",
                    serde_json::json!({ "text": "change" }),
                    None,
                )
                .await
                .expect("change notification call");
            assert!(
                state.mcp_display_tools(&server).await.1,
                "notification makes passive display incomplete"
            );
            let refreshed = state
                .mcp_list_tools(None, &server)
                .await
                .expect("tools/list should refresh after list_changed");
            assert!(refreshed.iter().any(|tool| tool.name == "dynamic"));
            assert!(!state.mcp_display_tools(&server).await.1);

            state.mcp_disconnect_all().await;
            let _ = std::fs::remove_file(&script);
        }

        #[tokio::test]
        async fn oauth_shaped_remote_error_is_not_replayed() {
            let script = write_fake_server();
            let marker = script.with_extension("calls");
            let state = test_app_state();
            let mut server = python_server(&script);
            server.env.insert(
                "KIVIO_CALL_MARKER".into(),
                marker.to_string_lossy().into_owned(),
            );
            let error = state
                .mcp_call_tool(
                    None,
                    &server,
                    "echo",
                    serde_json::json!({"text": "oauth-error"}),
                    None,
                )
                .await
                .expect_err("remote execution error must remain an error");
            assert!(error.contains("Auth required"), "{error}");
            assert_eq!(std::fs::read_to_string(&marker).unwrap().lines().count(), 1);
            assert_eq!(state.mcp_server_state(&server.id).await.handshake_count, 1);
            state.mcp_disconnect_all().await;
            let _ = std::fs::remove_file(script);
            let _ = std::fs::remove_file(marker);
        }

        #[tokio::test]
        async fn same_config_snapshot_survives_session_removal_and_restart() {
            let script = write_fake_server();
            let state = test_app_state();
            let usage_dir = state.usage_dir.clone();
            let server = python_server(&script);

            let tools = state
                .mcp_list_tools(None, &server)
                .await
                .expect("initial list ok");
            assert!(tools.iter().any(|tool| tool.name == "echo"));

            let evicted = state.mcp_reap_idle(Duration::ZERO).await;
            assert_eq!(evicted.len(), 1, "idle reap should remove the live session");
            assert!(state.mcp_test_sessions_empty().await);
            assert!(state
                .mcp_cached_tools(&server)
                .await
                .expect("snapshot survives session removal")
                .iter()
                .any(|tool| tool.name == "echo"));

            let restarted = crate::state::AppState::new_headless(
                crate::settings::Settings::default(),
                usage_dir.clone(),
            );
            assert!(restarted
                .mcp_cached_tools(&server)
                .await
                .expect("snapshot reloads across AppState restart")
                .iter()
                .any(|tool| tool.name == "echo"));

            let _ = std::fs::remove_file(&script);
            let _ = std::fs::remove_dir_all(&usage_dir);
        }

        #[tokio::test]
        async fn same_config_failure_reuses_snapshot_but_changed_config_does_not() {
            let script = write_fake_server();
            let state = test_app_state();
            let server = python_server(&script);

            state
                .mcp_list_tools(None, &server)
                .await
                .expect("initial list ok");
            state.mcp_disconnect_all().await;
            std::fs::remove_file(&script).expect("remove fake server to force same-config failure");

            state
                .mcp_list_tools(None, &server)
                .await
                .expect_err("same config should now fail to reconnect");
            assert!(state
                .mcp_cached_tools(&server)
                .await
                .expect("same config may use last-known schema")
                .iter()
                .any(|tool| tool.name == "echo"));
            assert!(
                state.mcp_unreachable_server_ids().await.is_empty(),
                "a failed server with a matching snapshot is degraded, not unreachable"
            );

            let mut changed = server.clone();
            changed.command = "kivio-definitely-missing-cmd".to_string();
            state
                .mcp_list_tools(None, &changed)
                .await
                .expect_err("changed config must fail independently");
            assert!(
                state.mcp_cached_tools(&changed).await.is_none(),
                "a schema from an old fingerprint must never leak into changed config"
            );
            assert_eq!(
                state.mcp_unreachable_server_ids().await,
                vec![server.id.clone()]
            );

            state.mcp_disconnect_all().await;
            let _ = std::fs::remove_dir_all(&state.usage_dir);
        }

        #[tokio::test]
        async fn discovery_failures_are_throttled_but_explicit_connect_can_retry() {
            let state = test_app_state();
            let server = stdio_server("kivio-definitely-missing-cmd", &[]);

            state
                .mcp_list_tools(None, &server)
                .await
                .expect_err("first discovery connect must fail");
            let session = state
                .mcp_test_session(&server.id)
                .await
                .expect("error session retained");
            assert_eq!(session.lock().await.consecutive_connect_failures, 1);

            let second = state
                .mcp_list_tools(None, &server)
                .await
                .expect_err("second discovery should honor cooldown");
            assert!(
                second.contains("cooling down"),
                "unexpected error: {second}"
            );
            assert_eq!(
                session.lock().await.consecutive_connect_failures,
                1,
                "cooldown must avoid another spawn/handshake attempt"
            );

            assert!(
                state.mcp_get_or_connect(None, &server).await.is_err(),
                "explicit path retries immediately and still fails"
            );
            assert_eq!(
                session.lock().await.consecutive_connect_failures,
                2,
                "explicit tool path must bypass discovery cooldown"
            );

            state.mcp_disconnect_all().await;
            let _ = std::fs::remove_dir_all(&state.usage_dir);
        }

        #[tokio::test]
        async fn never_connected_error_server_is_unreachable() {
            let state = test_app_state();
            let server = stdio_server("kivio-definitely-missing-cmd", &[]);
            assert!(
                state.mcp_get_or_connect(None, &server).await.is_err(),
                "missing command must fail"
            );
            assert!(state.mcp_cached_tools(&server).await.is_none());
            assert_eq!(
                state.mcp_unreachable_server_ids().await,
                vec![server.id.clone()]
            );
            state.mcp_disconnect_all().await;
            let _ = std::fs::remove_dir_all(&state.usage_dir);
        }

        #[test]
        fn discovery_retry_backoff_is_exponential_and_capped() {
            assert_eq!(discovery_retry_backoff(1), Duration::from_secs(2));
            assert_eq!(discovery_retry_backoff(2), Duration::from_secs(4));
            assert_eq!(discovery_retry_backoff(5), Duration::from_secs(32));
            assert_eq!(discovery_retry_backoff(6), Duration::from_secs(60));
            assert_eq!(discovery_retry_backoff(99), Duration::from_secs(60));
        }

        #[tokio::test]
        async fn reused_stdio_session_honors_increased_tool_timeout() {
            let script = write_fake_server();
            let state = test_app_state();
            state.update_settings_for_test(|settings| settings.chat_tools.tool_timeout_ms = 1_000);

            let mut server = python_server(&script);
            server
                .env
                .insert("KIVIO_DELAY_CALL_MS".to_string(), "2500".to_string());

            let err = state
                .mcp_call_tool(
                    None,
                    &server,
                    "echo",
                    serde_json::json!({ "text": "slow" }),
                    None,
                )
                .await
                .expect_err("1s timeout should fail on a 2.5s tool");
            assert!(
                err.contains("timed out"),
                "expected timeout error, got: {err}"
            );

            state.update_settings_for_test(|settings| settings.chat_tools.tool_timeout_ms = 5_000);
            let result = state
                .mcp_call_tool(
                    None,
                    &server,
                    "echo",
                    serde_json::json!({ "text": "slow" }),
                    None,
                )
                .await
                .expect("5s timeout should succeed on the same reused session");
            assert_eq!(result.content, "echo: slow");

            state.mcp_disconnect_all().await;
            let _ = std::fs::remove_file(&script);
        }
    }
}
