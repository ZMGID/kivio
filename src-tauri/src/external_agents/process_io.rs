//! Shared CLI process lifetime and bounded diagnostic output.
use std::process::Stdio;
use std::time::Duration;
use tokio::process::{Child, ChildStderr, Command};
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::time::timeout;
/// Owns the Unix CLI process group through normal exit, error and cancellation.
/// On Windows, taskkill /T is best effort while the leader is still live;
/// descendants surviving a normal leader exit need a separately validated Job
/// owner. Tokio's kill_on_drop alone only kills the direct child.
pub struct AgentChild {
    child: Option<Child>,
    group: Option<u32>,
}

impl From<Child> for AgentChild {
    fn from(child: Child) -> Self {
        let group = child.id();
        Self {
            child: Some(child),
            group,
        }
    }
}

impl std::ops::Deref for AgentChild {
    type Target = Child;
    fn deref(&self) -> &Child {
        self.child.as_ref().expect("live CLI child")
    }
}
impl std::ops::DerefMut for AgentChild {
    fn deref_mut(&mut self) -> &mut Child {
        self.child.as_mut().expect("live CLI child")
    }
}
impl AgentChild {
    pub async fn wait_with_output(mut self) -> std::io::Result<std::process::Output> {
        use tokio::io::AsyncReadExt;
        let stdout = self.stdout.take();
        let stderr = self.stderr.take();
        self.stdin.take();
        let read = |pipe: Option<tokio::process::ChildStdout>| async move {
            let mut bytes = Vec::new();
            if let Some(mut pipe) = pipe {
                pipe.read_to_end(&mut bytes).await?;
            }
            Ok::<_, std::io::Error>(bytes)
        };
        let read_stderr = async move {
            let mut bytes = Vec::new();
            if let Some(mut pipe) = stderr {
                pipe.read_to_end(&mut bytes).await?;
            }
            Ok::<_, std::io::Error>(bytes)
        };
        // Keep the Child inside its owner for the whole capture. In particular,
        // a cancelled Windows probe kills the tree before dropping its leader.
        let (status, stdout, stderr) = tokio::try_join!(self.wait(), read(stdout), read_stderr)?;
        Ok(std::process::Output {
            status,
            stdout,
            stderr,
        })
    }
    fn retire_group(&mut self) {
        if let Some(pid) = self.group.take() {
            #[cfg(windows)]
            if self.child.as_ref().and_then(Child::id) != Some(pid) {
                // A reaped leader no longer identifies its tree, and its PID may
                // be reused. Do not taskkill another process after normal exit.
                return;
            }
            crate::native_tools::kill_process_group(pid);
        }
    }
    pub fn start_kill(&mut self) -> std::io::Result<()> {
        self.retire_group();
        self.child.as_mut().expect("live CLI child").start_kill()
    }
    pub async fn wait(&mut self) -> std::io::Result<std::process::ExitStatus> {
        let result = self.child.as_mut().expect("live CLI child").wait().await;
        if result.is_ok() {
            self.retire_group();
        }
        result
    }
    pub fn try_wait(&mut self) -> std::io::Result<Option<std::process::ExitStatus>> {
        let result = self.child.as_mut().expect("live CLI child").try_wait();
        if matches!(result, Ok(Some(_))) {
            self.retire_group();
        }
        result
    }
}
impl Drop for AgentChild {
    fn drop(&mut self) {
        self.retire_group();
        if let Some(child) = self.child.as_mut() {
            let _ = child.start_kill();
        }
    }
}

/// Output capture with the same lifecycle owner as a persistent session.
/// Kept as a Command extension so existing probe argument/env builders remain.
pub trait CliProcessOutput {
    fn group_output(
        &mut self,
    ) -> impl std::future::Future<Output = std::io::Result<std::process::Output>> + Send;
}
impl CliProcessOutput for Command {
    async fn group_output(&mut self) -> std::io::Result<std::process::Output> {
        #[cfg(unix)]
        self.process_group(0);
        self.stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        AgentChild::from(self.spawn()?).wait_with_output().await
    }
}

/// 标记「当前进程正跑在某个 CLI 的会话里」的环境变量。Kivio 若是从 Claude Code 内启动
/// （开发时 `npm run dev`、或用户把 Kivio 挂在某个 agent 下），这些会**继承**到 Kivio 进程、
/// 再泄漏给它拉起的 CLI 子进程，让子进程误以为自己嵌套在另一个会话里而拒绝启动
/// （claude 报 "cannot be launched inside another session"）。
///
/// 本机实测：从 Claude Code 环境跑 `env | grep CLAUDE` 确实能看到
/// `CLAUDECODE` / `CLAUDE_CODE_ENTRYPOINT` / `CLAUDE_AGENT_SDK_VERSION` 都是 set 的。
///
/// **后两个是「宿主代管凭据」标记，比会话身份更致命**（真机验收时才发现）：
/// 宿主（Claude Code 桌面端）用 `CLAUDE_CODE_PROVIDER_MANAGED_BY_HOST=1` 告诉 claude
/// 「你的凭据由我在运行时注入」，并用 `CLAUDE_CODE_HOST_AUTH_ENV_VAR` 指明注入到哪个
/// 变量名（实测值 `ANTHROPIC_AUTH_TOKEN`）。子进程继承了这两个标记、却**拿不到宿主
/// 真正注入的那个 token**（它只存在于宿主进程内），于是 claude 认为自己处于代管模式、
/// 不再回退去读 `~/.claude/settings.json` 的 `env` 块 ⇒ 报 "Not logged in · Please run /login"。
///
/// 剥掉这两个后 claude 恢复自主认证，会自己读 settings.json 的 `env`（含用户配置的
/// `ANTHROPIC_AUTH_TOKEN` / `ANTHROPIC_BASE_URL`）。实测二分定位：单独清任何一个都无效，
/// 两个**必须一起**清；清掉后 `is_error=false`。
///
/// 只清这些**会话身份 / 代管标记**变量，绝不动 `ANTHROPIC_*` 本身——那些是用户配置的
/// 凭据与端点，子进程需要它们来认证。
const PARENT_SESSION_ENV_VARS: &[&str] = &[
    "CLAUDECODE",
    "CLAUDE_CODE_ENTRYPOINT",
    "CLAUDE_CODE_SSE_PORT",
    "CLAUDE_AGENT_SDK_VERSION",
    "CLAUDE_CODE_PROVIDER_MANAGED_BY_HOST",
    "CLAUDE_CODE_HOST_AUTH_ENV_VAR",
];

/// 所有拉起外部 CLI 的地方都必须先过这一手，把父会话身份变量剥掉。
///
/// 覆盖范围包括**探测**路径（version / auth / 模型 / 斜杠命令）——不只是跑轮次：
/// 探测同样是子进程，同样会被父会话变量干扰，且探测失败会被缓存下来
/// （`AVAILABILITY_CACHE_TTL` 600s），一次误判要 10 分钟才自愈。
pub fn strip_parent_session_env(command: &mut Command) -> &mut Command {
    for key in PARENT_SESSION_ENV_VARS {
        command.env_remove(key);
    }
    command
}

/// 结束一个外部 CLI 子进程**及其整棵进程树**。
///
/// 只 `start_kill()` 杀的是**直接子进程**：claude 会按用户的 `~/.claude.json` 把 MCP
/// 服务器作为自己的子进程拉起来，每轮取消一次就可能漏一批孤儿 MCP 进程留在系统里。
///
/// 复用 `native_tools::kill_process_group`（unix `killpg` SIGTERM→SIGKILL、
/// Windows `taskkill /T /F`），不写第二份（spec 第 2 条）。两步都做，缺一不可：
/// - Windows 的 `taskkill /T` 按父子关系遍历整棵树，能覆盖子进程；
/// - Unix CLI commands create an independent process group, so killpg reaches
///   descendants too; start_kill also covers the direct child.
///
/// 副作用提醒：杀进程导致的退出码在 Windows 上恒为 1（`TerminateProcess`），
/// 出口的「非零退出 = 失败」规则必须靠协议层完成标志豁免（spec 第 8b 条），
/// 否则会凭空造出失败气泡。
pub fn kill_agent_process_tree(child: &mut Child) {
    if let Some(pid) = child.id() {
        crate::native_tools::kill_process_group(pid);
    }
    let _ = child.start_kill();
}

/// Concurrently drain the child's stderr into a JoinHandle so a CLI that reports failures on
/// stderr doesn't (a) block on a full pipe while we read stdout, and (b) fail silently. Blank
/// lines are dropped and the buffer is capped at `STDERR_CAP_CHARS` (keeping the tail — the last
/// lines are usually the actual error). Call before the stdout read loop; await after `wait()`.
pub fn drain_stderr(child: &mut Child) -> tokio::task::JoinHandle<String> {
    spawn_stderr_tail(child.stderr.take())
}

/// Ring-buffer stderr drain for persistent sessions (N1): the CLI process is long-lived and its
/// stderr is `take()`n separately from stdout, so we can't use `drain_stderr(&mut Child)`. Spawns a
/// task that accumulates the tail (last `STDERR_CAP_CHARS`) until stderr hits EOF (i.e. the child
/// dies / is killed), then returns it. Join the handle on close / error to fold into diagnostics.
pub fn spawn_stderr_tail(stderr: Option<ChildStderr>) -> tokio::task::JoinHandle<String> {
    tokio::spawn(async move {
        match stderr {
            Some(stderr) => accumulate_tail(stderr, STDERR_CAP_CHARS).await,
            None => String::new(),
        }
    })
}

const STDERR_CAP_CHARS: usize = 8192;

/// How long to wait for stderr EOF after killing the child. `start_kill()` only
/// terminates the direct child; on Windows `dsh.cmd` → `node` often leaves the
/// grandchild holding the pipe, so a bare join blocks forever and wedgies the
/// dsh profile boot lock (every later dsh turn then spins with no error).
const STDERR_JOIN_TIMEOUT: Duration = Duration::from_secs(2);

/// Kill the child tree (so its stderr hits EOF) and join the drain task to
/// recover the stderr tail. Used by persistent sessions on a handshake/connect
/// error path (N1 + R2). Always returns — a stuck grandchild must not block
/// the caller.
pub async fn join_stderr_tail(
    child: &mut Child,
    stderr_tail: tokio::task::JoinHandle<String>,
) -> String {
    kill_agent_process_tree(child);
    match timeout(STDERR_JOIN_TIMEOUT, stderr_tail).await {
        Ok(Ok(tail)) => tail,
        Ok(Err(_)) => String::new(),
        Err(_) => String::new(),
    }
}

/// Append a drained stderr tail to an error message when non-empty (for R2 diagnostics).
pub fn fold_stderr(msg: String, stderr_tail: &str) -> String {
    if stderr_tail.trim().is_empty() {
        msg
    } else {
        format!("{msg}\nstderr: {}", stderr_tail.trim())
    }
}

/// Read lines from `reader` until EOF, keeping only the last `cap` characters (char-boundary safe).
/// Blank lines are dropped. Extracted from the drain tasks so the ring-buffer bound is unit-testable
/// without spawning a real process.
pub(super) async fn accumulate_tail<R: AsyncRead + Unpin>(reader: R, cap: usize) -> String {
    let mut lines = BufReader::new(reader).lines();
    let mut out = String::new();
    loop {
        let line = match lines.next_line().await {
            Ok(Some(line)) => line,
            Ok(None) => break,
            // 一行非法 UTF-8 不能让排空停下来：常驻进程的 stderr 从此再没人读，管道写满
            // （4–64KB）后子进程会阻塞在写 stderr 上 —— 正是这个任务要避免的那个死锁。
            // tokio 丢掉这一行后 reader 仍可用，接着读。**只对 InvalidData 继续**：其余
            // 错误（管道断了之类）是持续性的，continue 会变成空转。
            Err(err) if err.kind() == std::io::ErrorKind::InvalidData => continue,
            Err(_) => break,
        };
        if line.trim().is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&line);
        if out.chars().count() > cap {
            out = tail_chars(&out, cap);
        }
    }
    out
}

/// Keep the last `max_chars` characters of `value` (char-boundary safe).
pub fn tail_chars(value: &str, max_chars: usize) -> String {
    let chars: Vec<char> = value.chars().collect();
    let start = chars.len().saturating_sub(max_chars);
    chars[start..].iter().collect()
}
