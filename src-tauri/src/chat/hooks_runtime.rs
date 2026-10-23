// Shared lifecycle hook execution. Host adapters provide events and runtime.
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use serde::Serialize;

use crate::settings::HookDef;

/// 队列上限。溢出即丢弃（并只警告一次）——Hook 是旁路观测，宁可丢事件也不能反压 loop。
const MAX_QUEUED: usize = 64;

/// 对话生命周期事件。8 个，与 loop 的阶段一一对应，不发明新词。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookEvent {
    AgentStart,
    TurnStart,
    MessageStart,
    MessageEnd,
    ToolExecutionStart,
    ToolExecutionEnd,
    TurnEnd,
    AgentEnd,
}

impl HookEvent {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AgentStart => "agent_start",
            Self::TurnStart => "turn_start",
            Self::MessageStart => "message_start",
            Self::MessageEnd => "message_end",
            Self::ToolExecutionStart => "tool_execution_start",
            Self::ToolExecutionEnd => "tool_execution_end",
            Self::TurnEnd => "turn_end",
            Self::AgentEnd => "agent_end",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "agent_start" => Self::AgentStart,
            "turn_start" => Self::TurnStart,
            "message_start" => Self::MessageStart,
            "message_end" => Self::MessageEnd,
            "tool_execution_start" => Self::ToolExecutionStart,
            "tool_execution_end" => Self::ToolExecutionEnd,
            "turn_end" => Self::TurnEnd,
            "agent_end" => Self::AgentEnd,
            _ => return None,
        })
    }
}

/// Hook 脚本 stdin 收到的一行 JSON（HTTP Hook 的 POST body 同此）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct HookPayload<'a> {
    event: &'a str,
    hook_name: &'a str,
    conversation_id: &'a str,
    run_id: &'a str,
    message_id: &'a str,
    cwd: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    round: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_name: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<&'a str>,
    model: &'a str,
}

/// 每个 run 固定不变的上下文（会话/run/消息 id、工作目录、模型）。
pub struct RunContext {
    pub conversation_id: String,
    pub run_id: String,
    pub message_id: String,
    pub cwd: PathBuf,
    pub model: String,
}

struct HookJob {
    event: HookEvent,
    tool_name: Option<String>,
    tool_call_id: Option<String>,
    round: Option<u32>,
    /// 派发时的取消世代。worker 见到比当前世代旧的 job 直接丢弃。
    epoch: u64,
}

/// 失败上报通道。生产环境走 typed protocol hub；单测注入一个收集闭包，
/// 于是整个执行器不依赖 Tauri 运行时即可测试。
pub type FailureSink = Box<dyn Fn(serde_json::Value) + Send + Sync>;

struct Shared {
    ctx: RunContext,
    /// event → 该事件下启用的 Hook。
    by_event: HashMap<&'static str, Vec<HookDef>>,
    sink: FailureSink,
    /// 当前取消世代；`cancel()` 递增。
    epoch: AtomicU64,
    /// `cancel()` 叫醒正在 await 的 HTTP，好让 `select!` 丢掉 in-flight 请求。
    cancel_notify: tokio::sync::Notify,
    /// 在跑的脚本的进程组 leader pid（用于 `cancel()` 杀进程）。
    running_pid: Mutex<Option<u32>>,
    queued: AtomicUsize,
    overflow_warned: AtomicBool,
    /// 单测用的派发流水：记录每次 `dispatch` 的 `(event, tool_name)`，用于在 `loop_tests`
    /// 里断言事件配对，而不必为此起 8 个子进程。
    #[cfg(test)]
    log: Mutex<Vec<(&'static str, Option<String>)>>,
    /// 单测置 false：只记流水、不真的执行 Hook。
    execute: bool,
}

/// 按 run 创建的 Hook 调度器。串行消费，保证事件顺序。
pub struct HookDispatcher {
    tx: tokio::sync::mpsc::UnboundedSender<HookJob>,
    /// 生产路径不 join worker（detached，Drop 后自行排空退出）；只有单测的 `drain` 会等它。
    #[cfg_attr(not(test), allow(dead_code))]
    worker: tokio::task::JoinHandle<()>,
    shared: Arc<Shared>,
}

impl HookDispatcher {
    /// 这批配置里有没有可用的 Hook。调用侧据此在**构造入参之前**短路，
    /// 于是「没配 Hook」连那几个 id / model 字符串都不会分配（验收 6）。
    pub fn any_enabled(hooks: &[HookDef]) -> bool {
        hooks
            .iter()
            .any(|hook| hook.enabled && HookEvent::parse(&hook.event).is_some())
    }

    pub fn with_sink(
        hooks: &[HookDef],
        ctx: RunContext,
        sink: FailureSink,
        runtime: &tokio::runtime::Handle,
    ) -> Option<Self> {
        Self::build(hooks, ctx, sink, true, runtime)
    }

    /// 只记派发流水、不真的执行 Hook 的调度器（`loop_tests` 用它断言事件配对）。
    #[cfg(test)]
    pub(crate) fn recording() -> Self {
        let hook = HookDef {
            id: "rec".to_string(),
            name: "rec".to_string(),
            enabled: true,
            kind: "command".to_string(),
            script: "true".to_string(),
            timeout_ms: 1_000,
            ..Default::default()
        };
        // 8 个事件各挂一个，才能把每次 dispatch 都记下来（dispatch 对没配 Hook 的事件短路）。
        let hooks: Vec<HookDef> = [
            "agent_start",
            "turn_start",
            "message_start",
            "message_end",
            "tool_execution_start",
            "tool_execution_end",
            "turn_end",
            "agent_end",
        ]
        .iter()
        .map(|event| HookDef {
            event: event.to_string(),
            ..hook.clone()
        })
        .collect();
        Self::build(
            &hooks,
            RunContext {
                conversation_id: "conv".to_string(),
                run_id: "run".to_string(),
                message_id: "msg".to_string(),
                cwd: std::env::temp_dir(),
                model: "m".to_string(),
            },
            Box::new(|_| {}),
            false,
            &tokio::runtime::Handle::current(),
        )
        .expect("recording dispatcher")
    }

    /// 已派发事件流水（`(event, tool_name)`，按序）。
    #[cfg(test)]
    pub(crate) fn recorded(&self) -> Vec<(&'static str, Option<String>)> {
        self.shared
            .log
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .clone()
    }

    /// 取消世代。>0 即「本 run 期间至少 `cancel()` 过一次」，`loop_tests` 用它断言
    /// 取消真的传到了调度器（光看事件流水看不出来：取消路径的事件序列与正常路径同形）。
    #[cfg(test)]
    pub(crate) fn cancel_epoch(&self) -> u64 {
        self.shared.epoch.load(Ordering::SeqCst)
    }

    fn build(
        hooks: &[HookDef],
        mut ctx: RunContext,
        sink: FailureSink,
        execute: bool,
        runtime: &tokio::runtime::Handle,
    ) -> Option<Self> {
        let mut by_event: HashMap<&'static str, Vec<HookDef>> = HashMap::new();
        for hook in hooks.iter().filter(|hook| hook.enabled) {
            let Some(event) = HookEvent::parse(&hook.event) else {
                continue;
            };
            by_event
                .entry(event.as_str())
                .or_default()
                .push(hook.clone());
        }
        if by_event.is_empty() {
            return None;
        }

        // 会话工作目录是**懒创建**的（只有原生工具真的解析出 cwd 时才 mkdir），所以一个
        // 从没用过工具的普通会话，这个路径可能根本不存在 —— 直接拿去当 `current_dir`
        // 会让 spawn 以 ENOENT 失败，用户看到的是「hook 启动失败」而不是自己的脚本输出。
        // 落到临时目录（对齐设计里「解析不出则用临时目录」）而不是 mkdir：不能因为配了
        // 一个 Hook 就在工作区根下凭空生出空目录。
        if !ctx.cwd.is_dir() {
            ctx.cwd = std::env::temp_dir();
        }

        let shared = Arc::new(Shared {
            ctx,
            by_event,
            sink,
            epoch: AtomicU64::new(0),
            cancel_notify: tokio::sync::Notify::new(),
            running_pid: Mutex::new(None),
            queued: AtomicUsize::new(0),
            overflow_warned: AtomicBool::new(false),
            #[cfg(test)]
            log: Mutex::new(Vec::new()),
            execute,
        });
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<HookJob>();
        let worker_shared = Arc::clone(&shared);
        let worker = runtime.spawn(async move {
            while let Some(job) = rx.recv().await {
                worker_shared.queued.fetch_sub(1, Ordering::Relaxed);
                run_job(&worker_shared, job).await;
            }
        });
        Some(Self { tx, worker, shared })
    }

    /// 只是入队，永不阻塞 loop。
    pub fn dispatch(
        &self,
        event: HookEvent,
        tool_name: Option<&str>,
        round: Option<u32>,
        tool_call_id: Option<&str>,
    ) {
        if !self.shared.by_event.contains_key(event.as_str()) {
            return;
        }
        // 流水记在入队处（而非 worker 里）：断言的是 loop 的派发顺序，不是执行调度。
        #[cfg(test)]
        if let Ok(mut log) = self.shared.log.lock() {
            log.push((event.as_str(), tool_name.map(str::to_string)));
        }
        if !self.shared.execute {
            return;
        }
        if self.shared.queued.load(Ordering::Relaxed) >= MAX_QUEUED {
            if !self.shared.overflow_warned.swap(true, Ordering::Relaxed) {
                self.shared.report(
                    event.as_str(),
                    "",
                    &format!("Hook queue overflowed ({MAX_QUEUED}); further events are dropped"),
                );
            }
            return;
        }
        self.shared.queued.fetch_add(1, Ordering::Relaxed);
        let job = HookJob {
            event,
            tool_name: tool_name.map(str::to_string),
            tool_call_id: tool_call_id.map(str::to_string),
            round,
            epoch: self.shared.epoch.load(Ordering::SeqCst),
        };
        if self.tx.send(job).is_err() {
            self.shared.queued.fetch_sub(1, Ordering::Relaxed);
        }
    }

    /// 用户停止 / run 取消：丢弃排队中的 Hook，杀掉在跑的脚本。
    /// **不永久禁用**——`agent_end` 在取消后仍需触发（它由 Drop guard 派发），
    /// 所以这里只作废「此刻之前」入队的 job，之后新派发的照常执行。
    pub fn cancel(&self) {
        self.shared.epoch.fetch_add(1, Ordering::SeqCst);
        self.shared.cancel_notify.notify_waiters();
        let pid = self
            .shared
            .running_pid
            .lock()
            .ok()
            .and_then(|mut p| p.take());
        if let Some(pid) = pid {
            crate::native_tools::kill_process_group(pid);
        }
    }

    /// 关闭队列并等 worker 排空。单测用（生产路径靠 Drop：sender 落地 → 通道关闭 →
    /// worker 排空剩余事件后自然退出，所以最后的 `agent_end` 不会被吞掉）。
    #[cfg(test)]
    async fn drain(self) {
        let Self { tx, worker, .. } = self;
        drop(tx);
        let _ = worker.await;
    }
}

impl Shared {
    fn report(&self, event: &str, hook_name: &str, message: &str) {
        (self.sink)(serde_json::json!({
            "conversationId": self.ctx.conversation_id,
            "runId": self.ctx.run_id,
            "hookName": hook_name,
            "event": event,
            "message": message,
        }));
    }
}

async fn run_job(shared: &Shared, job: HookJob) {
    let Some(hooks) = shared.by_event.get(job.event.as_str()) else {
        return;
    };
    for hook in hooks {
        if shared.epoch.load(Ordering::SeqCst) != job.epoch {
            return;
        }
        let payload = HookPayload {
            event: job.event.as_str(),
            hook_name: &hook.name,
            conversation_id: &shared.ctx.conversation_id,
            run_id: &shared.ctx.run_id,
            message_id: &shared.ctx.message_id,
            cwd: shared.ctx.cwd.display().to_string(),
            round: job.round,
            tool_name: job.tool_name.as_deref(),
            tool_call_id: job.tool_call_id.as_deref(),
            model: &shared.ctx.model,
        };
        let body = match serde_json::to_string(&payload) {
            Ok(body) => body,
            Err(err) => {
                shared.report(job.event.as_str(), &hook.name, &format!("payload: {err}"));
                continue;
            }
        };
        let result = match hook.kind.as_str() {
            "http" => {
                tokio::select! {
                    biased;
                    _ = wait_cancelled(shared, job.epoch) => Err("hook cancelled".to_string()),
                    result = run_http_hook(hook, &body) => result,
                }
            }
            _ => run_command_hook(hook, &body, &payload, shared, job.epoch).await,
        };
        // 被 cancel 杀掉的脚本会以失败返回，那不是用户该看的错误 —— 世代已变则静默。
        if let Err(err) = result {
            if shared.epoch.load(Ordering::SeqCst) == job.epoch {
                shared.report(job.event.as_str(), &hook.name, &err);
            }
        }
    }
}

/// 执行 command 类 Hook：载荷 JSON 写 stdin，`KIVIO_*` env 注入，超时杀进程组。
async fn run_command_hook(
    hook: &HookDef,
    body: &str,
    payload: &HookPayload<'_>,
    shared: &Shared,
    job_epoch: u64,
) -> Result<(), String> {
    use tokio::io::AsyncWriteExt as _;

    let mut cmd = crate::native_tools::build_shell_command(&hook.script);
    cmd.current_dir(&payload.cwd)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .env("KIVIO_HOOK_EVENT", payload.event)
        .env("KIVIO_HOOK_NAME", payload.hook_name)
        .env("KIVIO_CONVERSATION_ID", payload.conversation_id)
        .env("KIVIO_RUN_ID", payload.run_id)
        .env("KIVIO_WORKDIR", &payload.cwd)
        .env("KIVIO_TOOL_NAME", payload.tool_name.unwrap_or(""))
        .env("KIVIO_TOOL_CALL_ID", payload.tool_call_id.unwrap_or(""))
        .kill_on_drop(true);
    #[cfg(windows)]
    {
        use crate::proc::NoConsoleWindow as _;
        cmd.no_console_window();
    }
    // setsid：让 `kill_process_group` 能连子孙一起杀（对齐 native_tools::shell）。
    #[cfg(unix)]
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }

    let mut child = cmd
        .spawn()
        .map_err(|err| format!("failed to start hook script: {err}"))?;
    let pid = child.id();
    // 登记 pid 供 `cancel()` 杀进程。登记后必须复检世代：cancel 可能恰好落在
    // 「上面 run_job 检查过世代」与「这里登记」之间 —— 那次 cancel 看到的是空槽位，
    // 于是这个刚起来的进程谁也不会杀（脚本是 `sleep 600` 就泄漏到 run 结束之后）。
    // 复检发现世代已变则自己收尾。
    if let Some(pid) = pid {
        if let Ok(mut slot) = shared.running_pid.lock() {
            *slot = Some(pid);
        }
        if shared.epoch.load(Ordering::SeqCst) != job_epoch {
            if let Ok(mut slot) = shared.running_pid.lock() {
                if *slot == Some(pid) {
                    *slot = None;
                }
            }
            crate::native_tools::kill_process_group(pid);
            return Err("hook cancelled".to_string());
        }
    }
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(body.as_bytes()).await;
        let _ = stdin.write_all(b"\n").await;
        let _ = stdin.shutdown().await;
    }

    let waited = tokio::time::timeout(
        std::time::Duration::from_millis(hook.timeout_ms),
        child.wait_with_output(),
    )
    .await;
    if let Ok(mut slot) = shared.running_pid.lock() {
        if *slot == pid {
            *slot = None;
        }
    }

    let output = match waited {
        Err(_) => {
            if let Some(pid) = pid {
                crate::native_tools::kill_process_group(pid);
            }
            return Err(format!("hook timed out after {}ms", hook.timeout_ms));
        }
        Ok(Err(err)) => return Err(format!("hook script failed: {err}")),
        Ok(Ok(output)) => output,
    };
    if output.status.success() {
        return Ok(());
    }
    let code = output
        .status
        .code()
        .map(|code| code.to_string())
        .unwrap_or_else(|| "signal".to_string());
    let stderr = String::from_utf8_lossy(&output.stderr);
    let detail = stderr.trim();
    if detail.is_empty() {
        Err(format!("hook exited with code {code}"))
    } else {
        Err(format!("hook exited with code {code}: {detail}"))
    }
}

/// 等到取消世代越过 `job_epoch`。先订阅 Notify 再读 epoch，避免 cancel 落在
/// 检查与 wait 之间而永久挂起。
async fn wait_cancelled(shared: &Shared, job_epoch: u64) {
    loop {
        let notified = shared.cancel_notify.notified();
        if shared.epoch.load(Ordering::SeqCst) != job_epoch {
            return;
        }
        notified.await;
    }
}

/// 执行 http 类 Hook。仅 2xx 视为成功。GET/HEAD 不带 body。
// ponytail: 载荷永远是 Hook 自己的 JSON —— `HookDef` 没有自定义 body 字段，
// 需要别的形状就用 command 类 Hook 加一行 curl。
async fn run_http_hook(hook: &HookDef, body: &str) -> Result<(), String> {
    let method = reqwest::Method::from_bytes(hook.method.as_bytes())
        .map_err(|_| format!("invalid HTTP method {}", hook.method))?;
    let send_body = method != reqwest::Method::GET && method != reqwest::Method::HEAD;
    // Loopback hooks are local IPC. Sending them through an environment/system proxy can both
    // leak the payload and make an otherwise healthy local integration depend on proxy health.
    let direct = reqwest::Url::parse(hook.url.trim())
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned))
        .is_some_and(|host| {
            host.eq_ignore_ascii_case("localhost")
                || host
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|ip| ip.is_loopback())
        });
    let client = if direct {
        crate::api::build_direct_http_client()
    } else {
        crate::api::build_http_client()
    };
    let mut request = client
        .request(method, hook.url.trim())
        .timeout(std::time::Duration::from_millis(hook.timeout_ms))
        .header("content-type", "application/json")
        .header("X-Kivio-Hook-Event", hook.event.as_str());
    if send_body {
        request = request.body(body.to_string());
    }
    for (key, value) in &hook.headers {
        request = request.header(key, value);
    }
    let response = request
        .send()
        .await
        .map_err(|err| format!("hook request failed: {err}"))?;
    let status = response.status();
    if status.is_success() {
        Ok(())
    } else {
        Err(format!("hook responded {status}"))
    }
}
