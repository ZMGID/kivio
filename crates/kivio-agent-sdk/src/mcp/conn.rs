use std::{
    collections::HashMap,
    error::Error as StdError,
    future::Future,
    process::Stdio,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};

use reqwest::header::{HeaderName, HeaderValue};
use rmcp::{
    model::{ClientCapabilities, ClientConfig, Implementation, ProtocolVersion},
    service::{NotificationContext, RoleClient, RunningService},
    transport::{
        streamable_http_client::StreamableHttpClientTransportConfig, StreamableHttpClientTransport,
        TokioChildProcess,
    },
    ClientHandler, ClientLifecycleMode, ClientServiceExt,
};
use tokio::process::{ChildStderr, Command};

use super::{McpClientConfig, McpTransport};

/// legacy 握手（`initialize`）里宣称的协议版本。
///
/// **刻意仍是 2025-06-18**：所有现存服务器走的都是这条路，保持它就等于「对既有用户
/// wire 上零变化」。规范的新特性由下面的 discover 路径承接，不靠改这一行。
const LEGACY_PROTOCOL_VERSION: ProtocolVersion = ProtocolVersion::V_2025_06_18;

/// 新协议（2026-07-28+）的生命周期：`server/discover`，没有 `initialize` 握手。
///
/// **只在 `initialize` 被对端以「没这个方法」拒掉之后才用**，见 `connect`。
fn discover_lifecycle() -> ClientLifecycleMode {
    ClientLifecycleMode::Discover {
        // 只有这两版有 `server/discover`；顺序即偏好。
        preferred_versions: vec![ProtocolVersion::V_2026_07_28, ProtocolVersion::V_2025_11_25],
    }
}

/// 对端以 JSON-RPC `-32601` / "method not found" 拒掉了这次请求。
pub fn is_method_not_found(err: &str) -> bool {
    let lower = err.to_ascii_lowercase();
    lower.contains("-32601") || lower.contains("method not found")
}

/// 这次握手失败，是不是因为对端**根本没有 `initialize` 这个方法**？
///
/// 规范 2026-07-28 把 `initialize` / `notifications/initialized` 整个删掉了，改成
/// `server/discover`。只实现新规范的服务器按 JSON-RPC 就该对 `initialize` 回 `-32601`
/// —— 这是「该换新协议再试一次」唯一可靠的信号。
///
/// **刻意只认这一个信号**：其他失败（服务器起不来、认证失败、超时）再试一次纯属浪费，
/// stdio 还要多起一个子进程。
fn suggests_new_protocol(err: &str) -> bool {
    is_method_not_found(err)
}

/// `tools/list` 的整体超时上限。工具清单不该慢过这个数，慢了就是服务器有问题。
pub const LIST_TOOLS_TIMEOUT: Duration = Duration::from_secs(30);

/// 握手（`initialize`）的超时上限。
///
/// rmcp 的 `legacy_startup` 等 `initialize` 响应是**裸 await**，没有内建超时；手写版
/// 一直是带超时的。少了它，一个「进程起来了但不回 initialize」的 server（npx 正在下包、
/// 脚本卡在读 stdin、二进制版本不匹配）会永久占住 `McpSession` 的会话锁，连带让退出钩子
/// 里的 `mcp_disconnect_all` 永远拿不到锁 ⇒ 应用关不掉。
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(30);

/// HTTP 保活用的 `ping` 超时。失败不应拖住空闲扫描。
pub const PING_TIMEOUT: Duration = Duration::from_secs(5);

/// 握手失败时，最多花这么久去把子进程 stderr 上的错误信息捞出来贴进报错。
const STDERR_PEEK_TIMEOUT: Duration = Duration::from_millis(300);
const STDERR_PEEK_BYTES: usize = 2048;

/// rmcp 服务句柄的具体类型 —— stdio 与 HTTP 共用同一个，所以上层不再需要
/// 按传输方式分叉。
pub type McpService = RunningService<RoleClient, KivioClientHandler>;

/// 一条活着的 MCP 连接。
pub struct Conn {
    pub service: McpService,
    /// 服务器每发一次 `notifications/tools/list_changed` 就 +1，上层据此决定
    /// 要不要重新 `tools/list`。
    pub tools_revision: Arc<AtomicU64>,
    /// stdio 才有；上层用它起 stderr 尾巴任务（连接失败时把最后几行贴进错误信息）。
    pub stderr: Option<ChildStderr>,
    /// stdio 子进程 pid。`RunningService` 之后就拿不到子进程句柄了，而测试要用它
    /// 判断「超时没有杀掉健康子进程」「退出后没留孤儿」。
    pub child_pid: Option<u32>,
}

/// 我们的 `ClientHandler`：除了上报 client info，唯一职责就是把
/// `tools/list_changed` 记成一次 revision 自增。
#[derive(Clone, Default)]
pub struct KivioClientHandler {
    tools_revision: Arc<AtomicU64>,
}

impl KivioClientHandler {
    pub fn revision_handle(&self) -> Arc<AtomicU64> {
        self.tools_revision.clone()
    }
}

impl ClientHandler for KivioClientHandler {
    fn get_info(&self) -> ClientConfig {
        ClientConfig::new(
            ClientCapabilities::default(),
            Implementation::new("Kivio", env!("CARGO_PKG_VERSION")),
        )
        .with_protocol_version(LEGACY_PROTOCOL_VERSION)
    }

    fn on_tool_list_changed(
        &self,
        _context: NotificationContext<RoleClient>,
    ) -> impl Future<Output = ()> + Send + '_ {
        self.tools_revision.fetch_add(1, Ordering::Relaxed);
        std::future::ready(())
    }
}

/// 建连并握手。`http` 只有 streamable_http 传输会用到。
///
/// **先走 `initialize`（与改动前逐字节一致）**，只有对端以 `-32601`「没这个方法」拒掉它时
/// 才换 `server/discover` 再试一次。
///
/// 为什么是这个顺序而不是反过来：规范 2026-07-28 删掉了 `initialize`，所以未来的服务器
/// 需要 discover —— 但**今天所有能连上的服务器都走 `initialize`**。先探 discover 的话，
/// 每一台现存服务器都要为一个还不存在的未来白付一次握手；更糟的是，对未知方法回「成功但空
/// result」而不是 `-32601` 的手写服务器（我们自己的假测试服务器就是这样）压根不会触发
/// rmcp 的回落，只能靠外层重试救回来 —— stdio 还得多起一个子进程。
///
/// 反过来之后：现存服务器零额外开销、wire 上零变化；只有新协议独占的服务器多付一次握手，
/// 而它们今天一台都没有。
///
/// 重建传输而不是复用：`serve_*` 按值吃掉 transport，而且第一次已经往连接上写过一条
/// `initialize` —— 同一条连接上再握手会把两次会话的报文搅在一起。stdio 因此换一个子进程
/// （旧的随 `kill_on_drop` 走）。
pub async fn connect(server: &McpClientConfig, http: &reqwest::Client) -> Result<Conn, String> {
    let first = match connect_with(server, http, ClientLifecycleMode::Initialize).await {
        Ok(conn) => return Ok(conn),
        Err(err) => err,
    };
    if !suggests_new_protocol(&first) {
        return Err(first);
    }
    match connect_with(server, http, discover_lifecycle()).await {
        Ok(conn) => Ok(conn),
        // 两次都失败：报**第一次**的错误。它才是对端对我们默认协议的真实反馈。
        Err(_) => Err(first),
    }
}

async fn connect_with(
    server: &McpClientConfig,
    http: &reqwest::Client,
    lifecycle: ClientLifecycleMode,
) -> Result<Conn, String> {
    let handler = KivioClientHandler::default();
    let tools_revision = handler.revision_handle();

    match &server.transport {
        McpTransport::StreamableHttp { .. } => {
            let transport = build_http(server, http)?;
            let service = handshake(handler.serve_with_lifecycle(transport, lifecycle)).await?;
            Ok(Conn {
                service,
                tools_revision,
                stderr: None,
                child_pid: None,
            })
        }
        McpTransport::Stdio { .. } => {
            let (transport, mut stderr) = build_stdio(server)?;
            let child_pid = transport.id();
            let service = match handshake(handler.serve_with_lifecycle(transport, lifecycle)).await
            {
                Ok(service) => service,
                Err(err) => {
                    // 握手失败的 stdio server 往往**只在 stderr 上说话**（缺依赖、认证失败、
                    // 版本不对）。`spawn_stderr_tail` 只在握手成功后才挂上，所以这里必须自己
                    // 捞一段，否则用户只看到一句干巴巴的 "Failed to start MCP server X"。
                    let detail = peek_stderr(stderr.as_mut()).await;
                    return Err(if detail.is_empty() {
                        err
                    } else {
                        format!(
                            "{err}
{detail}"
                        )
                    });
                }
            };
            Ok(Conn {
                service,
                tools_revision,
                stderr,
                child_pid,
            })
        }
    }
}

/// 握手，带超时。超时按 `ServiceError::Timeout` 归一，走 `classify_error` 的超时分支。
///
/// 收的是 future 而不是 transport —— stdio 与 HTTP 的 `IntoTransport` 错误类型不同，
/// 泛型化会把两边的 bound 全拖进来，不值当。
async fn handshake<F, E>(serve: F) -> Result<McpService, String>
where
    F: Future<Output = Result<McpService, E>>,
    E: StdError,
{
    match tokio::time::timeout(HANDSHAKE_TIMEOUT, serve).await {
        Ok(Ok(service)) => Ok(service),
        Ok(Err(err)) => Err(classify_error("connect", &err)),
        Err(_) => Err(classify_error(
            "connect",
            &rmcp::ServiceError::Timeout {
                timeout: HANDSHAKE_TIMEOUT,
            },
        )),
    }
}

/// 尽力从 stderr 上读一小段（有多少读多少，不等进程退出）。
async fn peek_stderr(stderr: Option<&mut ChildStderr>) -> String {
    let Some(stderr) = stderr else {
        return String::new();
    };
    let mut buf = vec![0u8; STDERR_PEEK_BYTES];
    let read = tokio::time::timeout(STDERR_PEEK_TIMEOUT, async {
        tokio::io::AsyncReadExt::read(stderr, &mut buf).await
    })
    .await;
    match read {
        Ok(Ok(n)) if n > 0 => String::from_utf8_lossy(&buf[..n]).trim().to_string(),
        _ => String::new(),
    }
}

// rmcp drops the wrapper after a successful graceful wait. ProcessGroup itself
// does not kill surviving descendants then; KillOnDrop only owns the leader.
#[cfg(unix)]
#[derive(Debug)]
struct CleanupProcessGroup;

#[cfg(unix)]
impl process_wrap::tokio::CommandWrapper for CleanupProcessGroup {
    fn wrap_child(
        &mut self,
        inner: Box<dyn process_wrap::tokio::ChildWrapper>,
        _core: &process_wrap::tokio::CommandWrap,
    ) -> std::io::Result<Box<dyn process_wrap::tokio::ChildWrapper>> {
        Ok(Box::new(CleanupProcessGroupChild {
            inner: Some(inner),
            cleaned: false,
        }))
    }
}

#[cfg(unix)]
#[derive(Debug)]
struct CleanupProcessGroupChild {
    inner: Option<Box<dyn process_wrap::tokio::ChildWrapper>>,
    cleaned: bool,
}

#[cfg(unix)]
impl CleanupProcessGroupChild {
    fn cleanup(&mut self) {
        if self.cleaned {
            return;
        }
        // The inner ProcessGroupChild retains its PGID after the leader is
        // reaped. Clean immediately at wait completion, never via a dead PID.
        if let Some(inner) = &self.inner {
            if let Err(err) = inner.signal(libc::SIGKILL) {
                if err.raw_os_error() != Some(libc::ESRCH) {
                    eprintln!("Failed to clean MCP process group: {err}");
                }
            }
        }
        self.cleaned = true;
    }
}

#[cfg(unix)]
impl Drop for CleanupProcessGroupChild {
    fn drop(&mut self) {
        self.cleanup();
    }
}

#[cfg(unix)]
impl process_wrap::tokio::ChildWrapper for CleanupProcessGroupChild {
    fn inner(&self) -> &dyn process_wrap::tokio::ChildWrapper {
        self.inner.as_deref().expect("owned MCP child")
    }

    fn inner_mut(&mut self) -> &mut dyn process_wrap::tokio::ChildWrapper {
        self.inner.as_deref_mut().expect("owned MCP child")
    }

    fn into_inner(mut self: Box<Self>) -> Box<dyn process_wrap::tokio::ChildWrapper> {
        self.cleanup();
        self.inner.take().expect("owned MCP child")
    }

    fn wait(
        &mut self,
    ) -> std::pin::Pin<
        Box<dyn Future<Output = std::io::Result<std::process::ExitStatus>> + Send + '_>,
    > {
        Box::pin(async {
            let status = self.inner_mut().wait().await?;
            self.cleanup();
            Ok(status)
        })
    }

    fn try_wait(&mut self) -> std::io::Result<Option<std::process::ExitStatus>> {
        let status = self.inner_mut().try_wait()?;
        if status.is_some() {
            self.cleanup();
        }
        Ok(status)
    }
}

fn build_stdio(
    server: &McpClientConfig,
) -> Result<(TokioChildProcess, Option<ChildStderr>), String> {
    let McpTransport::Stdio {
        command: program,
        args,
        cwd,
        env,
    } = &server.transport
    else {
        return Err("MCP transport is not stdio".into());
    };
    if program.trim().is_empty() {
        return Err("MCP server command is empty".into());
    }
    let mut command =
        rmcp::transport::which_command(program).unwrap_or_else(|_| Command::new(program));
    command.args(args);
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    command.envs(clean_env(env));
    command.kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000);

    let mut wrapped = process_wrap::tokio::CommandWrap::from(command);
    #[cfg(unix)]
    wrapped
        .wrap(process_wrap::tokio::ProcessGroup::leader())
        .wrap(CleanupProcessGroup);
    #[cfg(windows)]
    {
        // JobObject overwrites raw Command flags; its CreationFlags wrapper
        // preserves CREATE_NO_WINDOW while adding temporary CREATE_SUSPENDED.
        // Assign the public u32 to avoid coupling the two windows crate versions.
        let mut flags = process_wrap::tokio::CreationFlags(Default::default());
        flags.0 .0 = 0x08000000;
        wrapped.wrap(flags).wrap(process_wrap::tokio::JobObject);
    }
    wrapped.wrap(process_wrap::tokio::KillOnDrop);

    TokioChildProcess::builder(wrapped)
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| format!("Failed to start MCP server: {err}"))
}

fn build_http(
    server: &McpClientConfig,
    http: &reqwest::Client,
) -> Result<StreamableHttpClientTransport<reqwest::Client>, String> {
    let McpTransport::StreamableHttp { url, headers } = &server.transport else {
        return Err("MCP transport is not Streamable HTTP".into());
    };
    let url = url.trim();
    if url.is_empty() {
        return Err("MCP server url is empty".into());
    }
    let mut config = StreamableHttpClientTransportConfig::with_uri(url);
    config.custom_headers = custom_headers(headers)?;
    Ok(StreamableHttpClientTransport::with_client(
        http.clone(),
        config,
    ))
}

pub(crate) fn clean_env(env: &HashMap<String, String>) -> Vec<(String, String)> {
    env.iter()
        .filter_map(|(key, value)| {
            let key = key.trim();
            if key.is_empty() {
                None
            } else {
                Some((key.to_string(), value.clone()))
            }
        })
        .collect()
}

fn custom_headers(
    headers: &HashMap<String, String>,
) -> Result<HashMap<HeaderName, HeaderValue>, String> {
    let mut out = HashMap::new();
    for (key, value) in headers {
        let key = key.trim();
        if key.is_empty() {
            continue;
        }
        let name = HeaderName::from_bytes(key.as_bytes())
            .map_err(|err| format!("Invalid MCP HTTP header {key}: {err}"))?;
        let value = HeaderValue::from_str(value.trim())
            .map_err(|err| format!("Invalid MCP HTTP header value for {key}: {err}"))?;
        out.insert(name, value);
    }
    Ok(out)
}

/// 把 rmcp 的错误变成我们的错误串，并在需要授权时加上 `OAUTH_REQUIRED:` 前缀
/// —— 设置页据此引导用户走 OAuth，而不是把裸 401 抛出去。
///
/// ponytail: 走 Display + source 链的文本匹配，而不是 downcast。真正的 HTTP 错误
/// 被 `ServiceError::TransportSend(DynamicTransportError)` 装成
/// `Box<dyn Error>`，要 downcast 得先拼出 `WorkerTransport<StreamableHttpClientWorker<
/// reqwest::Client>>` 这串泛型，不值当。下面的单测用真的 rmcp 错误值构造，
/// 所以 rmcp 改文案会让测试红掉，而不是静默回退。
pub fn classify_error<E: StdError>(context: &str, err: &E) -> String {
    let chain = error_chain(err);
    let mut base = format!("MCP {context} failed: {chain}");
    if is_timeout(&chain) {
        // rmcp 只说 "request timeout after PT1S"，但用户需要知道的是：请求已被取消
        // 通知给服务器，可服务器那边到底执行没执行不知道，我们也不会替他重试。
        // 手写版一直是这么说的，别丢。
        base.push_str("; timed out — request outcome is unknown and was not retried");
    }
    if requires_oauth(&chain) {
        format!("OAUTH_REQUIRED: {base}")
    } else {
        base
    }
}

/// 这条错误要不要走 OAuth 补救（刷新 token / 引导重新授权）。
///
/// 同时认 `OAUTH_REQUIRED:` 前缀（本函数自己打的）和尚未分类的 rmcp 原文，
/// 这样连接池路径和一次性 `call_tool_once` 都能在 401 后强制刷新。
pub fn is_oauth_error(err: &str) -> bool {
    let trimmed = err.trim();
    trimmed.starts_with("OAUTH_REQUIRED:") || requires_oauth(trimmed)
}

fn is_timeout(chain: &str) -> bool {
    chain.to_ascii_lowercase().contains("request timeout after")
}

fn error_chain<E: StdError>(err: &E) -> String {
    let mut parts = vec![err.to_string()];
    let mut source: Option<&(dyn StdError + 'static)> = err.source();
    while let Some(cause) = source {
        let text = cause.to_string();
        if !parts.iter().any(|part| part == &text) {
            parts.push(text);
        }
        source = cause.source();
    }
    parts.join(": ")
}

fn requires_oauth(chain: &str) -> bool {
    let lower = chain.to_ascii_lowercase();
    // `AuthRequiredError` 的 Display 是 "authorization required: <WWW-Authenticate 原文>"。
    // 只有 Bearer 质询才该引导用户走 OAuth —— rmcp 对**任何** 401 + WWW-Authenticate 都返回
    // AuthRequired（不看 scheme），而 Basic / Digest 那种自建服务器压根没有授权服务器，
    // 引导过去只会让用户在一个走不通的流程里打转。手写版是显式排掉非 Bearer 的。
    const AUTH_REQUIRED: &str = "authorization required:";
    if let Some(idx) = lower.find(AUTH_REQUIRED) {
        let challenge = lower[idx + AUTH_REQUIRED.len()..].trim();
        return challenge.is_empty() || challenge.contains("bearer");
    }
    // WorkerTransport 把 AuthRequiredError 包成
    // "Auth required, when send initialize request"，质询串经常不在 Display 链上。
    // 远程 OAuth MCP 的 401 就是这句，旧匹配漏掉之后既不标 OAUTH_REQUIRED、也不触发刷新。
    if lower.contains("auth required") {
        return !(lower.contains("basic realm") || lower.contains("digest realm"));
    }
    // 403 + insufficient_scope：重新授权（申请更多 scope）确实是正确的补救，保持引导。
    if lower.contains("insufficient scope") {
        return true;
    }
    // 401 但没带 WWW-Authenticate 质询头时 rmcp 不走 AuthRequired，而是落到
    // UnexpectedServerResponse("HTTP 401 ...")。手写版对这种也引导 OAuth，保持一致。
    lower.contains("http 401")
}

#[cfg(test)]
mod tests {
    use rmcp::transport::streamable_http_client::{
        AuthRequiredError, InsufficientScopeError, StreamableHttpError,
    };
    use std::borrow::Cow;
    use std::error::Error as StdError;

    use super::*;

    #[cfg(unix)]
    #[tokio::test]
    async fn stdio_graceful_shutdown_terminates_descendants_after_leader_exits_on_eof() {
        let directory = std::env::temp_dir().join(format!("kivio-mcp-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let pid_file = directory.join("descendant.pid");
        let server = McpClientConfig { transport: McpTransport::Stdio {
            command: "/bin/sh".into(),
            args: vec![
                "-c".into(),
                "sleep 60 </dev/null >/dev/null 2>&1 & echo $! > \"$1\"; while read line; do :; done; exit 0".into(),
                "sh".into(),
                pid_file.to_string_lossy().into_owned(),
            ],
            cwd: None, env: HashMap::new(),
        }};
        let (mut transport, _) = build_stdio(&server).unwrap();
        struct Cleanup(i32);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                if self.0 > 0 {
                    unsafe { libc::kill(-self.0, libc::SIGKILL) };
                }
            }
        }
        let mut cleanup = Cleanup(transport.id().unwrap() as i32);
        let descendant = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if let Ok(pid) = std::fs::read_to_string(&pid_file) {
                    if let Ok(pid) = pid.trim().parse::<u32>() {
                        break pid;
                    }
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        tokio::time::timeout(Duration::from_secs(4), transport.graceful_shutdown())
            .await
            .unwrap()
            .unwrap();
        // Keep the transport alive: cleanup belongs to graceful shutdown, not a
        // later Conn/transport drop. A reparented zombie is no longer executing.
        let exited = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let state = std::process::Command::new("ps")
                    .args(["-p", &descendant.to_string(), "-o", "stat="])
                    .output()
                    .unwrap();
                let state = String::from_utf8_lossy(&state.stdout);
                let state = state.trim();
                if state.is_empty() || state.starts_with('Z') {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await;
        assert!(
            exited.is_ok(),
            "graceful shutdown left a running descendant"
        );
        cleanup.0 = 0;
        let _ = std::fs::remove_dir_all(directory);
    }

    type TestHttpError = StreamableHttpError<std::io::Error>;

    #[test]
    fn oauth_prefix_on_401_with_bearer_challenge() {
        let err: TestHttpError = StreamableHttpError::AuthRequired(AuthRequiredError::new(
            "Bearer realm=\"mcp\"".to_string(),
        ));
        assert!(classify_error("connect", &err).starts_with("OAUTH_REQUIRED: "));
    }

    #[test]
    fn oauth_prefix_on_bare_401_without_challenge_header() {
        // rmcp 对无质询头的 401 走 UnexpectedServerResponse，串里带状态码。
        let err: TestHttpError = StreamableHttpError::UnexpectedServerResponse(Cow::Owned(
            "HTTP 401 Unauthorized: {\"error\":\"unauthorized\"}".to_string(),
        ));
        assert!(classify_error("connect", &err).starts_with("OAUTH_REQUIRED: "));
    }

    #[test]
    fn oauth_prefix_on_403_insufficient_scope() {
        let err: TestHttpError =
            StreamableHttpError::InsufficientScope(InsufficientScopeError::new(
                "Bearer error=\"insufficient_scope\", scope=\"files:read\"".to_string(),
                Some("files:read".to_string()),
            ));
        assert!(classify_error("connect", &err).starts_with("OAUTH_REQUIRED: "));
    }

    #[test]
    fn no_oauth_prefix_on_401_with_non_bearer_challenge() {
        // 手写版有这条断言（`client.rs::classify_http_error_flags_oauth_on_401_bearer`），
        // 换 rmcp 时丢了。Basic 认证的自建 server 没有授权服务器，引导 OAuth 是死路。
        let err: TestHttpError = StreamableHttpError::AuthRequired(AuthRequiredError::new(
            "Basic realm=\"internal\"".to_string(),
        ));
        let text = classify_error("connect", &err);
        assert!(!text.starts_with("OAUTH_REQUIRED: "), "{text}");
        // 但错误本身还得带出质询串，用户要知道该填什么。
        assert!(text.to_ascii_lowercase().contains("basic"), "{text}");
    }

    #[test]
    fn no_oauth_prefix_on_session_expired_or_500() {
        let expired: TestHttpError = StreamableHttpError::SessionExpired;
        assert!(!classify_error("call", &expired).starts_with("OAUTH_REQUIRED: "));

        let server_error: TestHttpError = StreamableHttpError::UnexpectedServerResponse(
            Cow::Owned("HTTP 500 Internal Server Error: boom".to_string()),
        );
        let text = classify_error("call", &server_error);
        assert!(!text.starts_with("OAUTH_REQUIRED: "));
        assert!(text.contains("500"));
    }

    #[test]
    fn no_oauth_prefix_on_transport_closed() {
        let err: TestHttpError = StreamableHttpError::TransportChannelClosed;
        assert!(!classify_error("call", &err).starts_with("OAUTH_REQUIRED: "));
    }

    #[test]
    fn timeout_says_outcome_is_unknown_and_not_retried() {
        // 这句话是给用户看的：超时的工具调用可能已经在服务器上执行了，我们没有重试。
        // 光说 "request timeout after PT1S" 传达不到这一点。
        let err = rmcp::ServiceError::Timeout {
            timeout: Duration::from_secs(1),
        };
        let text = classify_error("tools/call", &err);
        assert!(text.contains("outcome is unknown"), "{text}");
        assert!(text.contains("was not retried"), "{text}");
        assert!(!text.starts_with("OAUTH_REQUIRED: "));
    }

    #[test]
    fn error_chain_includes_source_detail() {
        let err: TestHttpError = StreamableHttpError::AuthRequired(AuthRequiredError::new(
            "Bearer realm=\"notion\"".to_string(),
        ));
        // 顶层 Display 只有 "Auth required"，质询串在 source 里 —— 必须带出来，
        // 否则用户看不到是哪个 realm 要授权。
        assert!(classify_error("connect", &err).contains("notion"));
    }

    #[test]
    fn oauth_prefix_on_worker_transport_auth_required_display() {
        // TinyFish 实测：WorkerTransport 只把 "Auth required, when send initialize
        // request" 写进 Display，质询串不在 source 链上。旧匹配只认
        // "authorization required:"，于是既不打 OAUTH_REQUIRED、也不触发刷新。
        #[derive(Debug)]
        struct WorkerAuthErr;
        impl std::fmt::Display for WorkerAuthErr {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(
                    f,
                    "Send message error Transport [rmcp::transport::worker::WorkerTransport<rmcp::transport::streamable_http_client::StreamableHttpClientWorker<reqwest::async_impl::client::Client>>] error: Auth required, when send initialize request"
                )
            }
        }
        impl StdError for WorkerAuthErr {}
        let text = classify_error("connect", &WorkerAuthErr);
        assert!(text.starts_with("OAUTH_REQUIRED: "), "{text}");
        assert!(is_oauth_error(&text));
        assert!(is_oauth_error(
            "MCP connect failed: Send message error Transport [...] error: Auth required, when send initialize request"
        ));
    }

    /// 「换新协议再试一次」的判据只认 `-32601`。放宽会让每一次普通的连接失败都多跑一次
    /// 握手（stdio 还多起一个子进程）；收紧到认不出来，则只支持 2026-07-28 的服务器永远连不上。
    #[test]
    fn only_method_not_found_suggests_the_new_protocol() {
        assert!(suggests_new_protocol(
            "MCP connect failed: Json rpc error: code -32601, message Method not found"
        ));
        assert!(suggests_new_protocol("server said: Method Not Found"));

        // 这些都不该触发重试。
        assert!(!suggests_new_protocol(
            "Failed to start MCP server X: No such file"
        ));
        assert!(!suggests_new_protocol(
            "OAUTH_REQUIRED: MCP connect failed: authorization required: Bearer"
        ));
        assert!(!suggests_new_protocol(
            "MCP connect failed: request timeout after PT30S"
        ));
        assert!(!suggests_new_protocol(
            "code -32600, message Invalid request"
        ));
    }

    /// legacy 路径宣称的版本不能被顺手改掉：所有现存服务器走的都是这条路，
    /// 改它等于对既有用户变更 wire 行为。
    #[test]
    fn legacy_handshake_still_advertises_2025_06_18() {
        assert_eq!(LEGACY_PROTOCOL_VERSION, ProtocolVersion::V_2025_06_18);
        let info = KivioClientHandler::default().get_info();
        assert_eq!(info.protocol_version, ProtocolVersion::V_2025_06_18);
    }

    /// discover 只请求真正带 `server/discover` 的版本，且新的在前。
    #[test]
    fn discover_prefers_the_newest_protocol() {
        let ClientLifecycleMode::Discover { preferred_versions } = discover_lifecycle() else {
            panic!("discover_lifecycle 必须是 Discover 模式");
        };
        assert_eq!(
            preferred_versions,
            vec![ProtocolVersion::V_2026_07_28, ProtocolVersion::V_2025_11_25]
        );
    }

    #[test]
    fn clean_env_drops_blank_keys() {
        let env = HashMap::from([
            ("  ".to_string(), "ignored".to_string()),
            (" TOKEN ".to_string(), "v".to_string()),
        ]);
        let out = clean_env(&env);
        assert_eq!(out, vec![("TOKEN".to_string(), "v".to_string())]);
    }

    fn probe_config(text: &str) -> McpClientConfig {
        #[cfg(windows)]
        let (command, args) = ("cmd", vec!["/c".into(), format!("echo {text} 1>&2")]);
        #[cfg(not(windows))]
        let (command, args) = ("sh", vec!["-c".into(), format!("echo {text} 1>&2")]);
        McpClientConfig {
            transport: McpTransport::Stdio {
                command: command.into(),
                args,
                cwd: None,
                env: HashMap::new(),
            },
        }
    }

    #[tokio::test]
    async fn build_stdio_pipes_stderr_and_reads_it() {
        let (transport, stderr) = build_stdio(&probe_config("boom")).expect("spawn");
        let mut stderr = stderr.expect("piped stderr");
        let mut buf = String::new();
        tokio::io::AsyncReadExt::read_to_string(&mut stderr, &mut buf)
            .await
            .unwrap();
        assert!(buf.contains("boom"));
        drop(transport);
    }

    #[test]
    fn empty_command_is_rejected_before_spawn() {
        let config = McpClientConfig {
            transport: McpTransport::Stdio {
                command: String::new(),
                args: Vec::new(),
                cwd: None,
                env: HashMap::new(),
            },
        };
        assert!(build_stdio(&config).is_err());
    }

    #[tokio::test]
    async fn failed_handshake_reports_stderr_reason() {
        let Err(error) =
            connect(&probe_config("missing-dependency"), &reqwest::Client::new()).await
        else {
            panic!("handshake must fail");
        };
        assert!(error.contains("missing-dependency"), "{error}");
    }

    #[test]
    fn custom_headers_maps_authorization_and_rejects_bad_names() {
        let ok = custom_headers(&HashMap::from([(
            "Authorization".to_string(),
            "Bearer abc".to_string(),
        )]))
        .expect("Authorization 不是 rmcp 的保留头，必须能过");
        assert_eq!(
            ok.get(&HeaderName::from_static("authorization")).unwrap(),
            "Bearer abc"
        );

        assert!(custom_headers(&HashMap::from([(
            "bad header".to_string(),
            "v".to_string()
        )]))
        .is_err());
    }
}
