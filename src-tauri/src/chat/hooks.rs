//! 对话生命周期 Hooks：在内置 agent loop 的 8 个事件点执行用户配置的 Shell 脚本
//! 或 HTTP 请求（任务 07-28-hooks）。
//!
//! **调度端到端在 Rust**。生命周期事件天然发生在 `chat/agent/loop_.rs`，把执行链放到
//! 前端就要为每个事件加一条 Tauri 事件 + 一次 IPC 往返 + 一套跨进程 scope 取消注册表。
//! 前端只负责配置 UI 和展示失败警告（typed protocol event）。
//!
//! Hook 一律 **fire-and-forget**：不能拒绝工具调用、不能改写参数、不能强制模型继续。
//! 失败只上报警告，绝不打断对话。

use tauri::AppHandle;
include!("hooks_runtime.rs");

impl HookDispatcher {
    /// 无启用 Hook 时返回 `None` —— 调用侧零开销（不构造载荷、不进队列）。
    pub fn new(
        app: AppHandle,
        hooks: &[HookDef],
        conversation_id: String,
        run_id: String,
        message_id: String,
        cwd: PathBuf,
        model: String,
    ) -> Option<Self> {
        let sink_run_id = run_id.clone();
        let sink_conversation_id = conversation_id.clone();
        let sink: FailureSink = Box::new(move |payload| {
            crate::chat::protocol::emit_hook_failed(
                &app,
                payload
                    .get("conversationId")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or(&sink_conversation_id),
                &sink_run_id,
                payload
                    .get("hookName")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                payload
                    .get("event")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                payload
                    .get("message")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
            );
        });
        Self::with_sink(
            hooks,
            RunContext {
                conversation_id,
                run_id,
                message_id,
                cwd,
                model,
            },
            sink,
            tauri::async_runtime::handle().inner(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc;
    use std::sync::{Arc, Mutex};

    /// 自清理的临时目录（本 crate 无 `tempfile` 依赖，沿用 shell.rs 的 temp_dir+uuid 写法）。
    #[cfg(unix)]
    struct TempDir(PathBuf);
    #[cfg(unix)]
    impl TempDir {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("kivio_hook_{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&path).expect("mkdir temp");
            Self(path)
        }
        fn path(&self) -> &std::path::Path {
            &self.0
        }
    }
    #[cfg(unix)]
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn command_hook(script: &str) -> HookDef {
        HookDef {
            id: "h1".to_string(),
            name: "probe".to_string(),
            event: "agent_end".to_string(),
            enabled: true,
            kind: "command".to_string(),
            script: script.to_string(),
            timeout_ms: 10_000,
            ..Default::default()
        }
    }

    fn ctx(cwd: PathBuf) -> RunContext {
        RunContext {
            conversation_id: "conv-1".to_string(),
            run_id: "run-1".to_string(),
            message_id: "msg-1".to_string(),
            cwd,
            model: "prov:model-x".to_string(),
        }
    }

    /// `(dispatcher, failure receiver)`
    fn dispatcher(
        hooks: &[HookDef],
        cwd: PathBuf,
    ) -> Option<(HookDispatcher, mpsc::Receiver<serde_json::Value>)> {
        let (tx, rx) = mpsc::channel();
        let sink: FailureSink = Box::new(move |payload| {
            let _ = tx.send(payload);
        });
        HookDispatcher::with_sink(hooks, ctx(cwd), sink, &tokio::runtime::Handle::current())
            .map(|dispatcher| (dispatcher, rx))
    }

    #[test]
    fn new_returns_none_without_enabled_hooks() {
        // 验收 6：无 Hook / 全禁用 → 调用侧零开销。
        assert!(dispatcher(&[], std::env::temp_dir()).is_none());
        let mut disabled = command_hook("true");
        disabled.enabled = false;
        assert!(dispatcher(&[disabled], std::env::temp_dir()).is_none());
        // 事件名非法的条目也不算数（sanitize 之外的第二道防线）。
        let mut bogus = command_hook("true");
        bogus.event = "not_an_event".to_string();
        assert!(dispatcher(&[bogus], std::env::temp_dir()).is_none());
    }

    #[test]
    fn parse_round_trips_all_events() {
        for event in [
            HookEvent::AgentStart,
            HookEvent::TurnStart,
            HookEvent::MessageStart,
            HookEvent::MessageEnd,
            HookEvent::ToolExecutionStart,
            HookEvent::ToolExecutionEnd,
            HookEvent::TurnEnd,
            HookEvent::AgentEnd,
        ] {
            assert_eq!(HookEvent::parse(event.as_str()), Some(event));
        }
        assert_eq!(HookEvent::parse("agent_started"), None);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn script_receives_stdin_json_and_env() {
        let dir = TempDir::new();
        let out = dir.path().join("payload.json");
        let env_out = dir.path().join("env.txt");
        let hook = command_hook(&format!(
            "cat > {out}; printf '%s|%s|%s|%s|%s' \"$KIVIO_HOOK_EVENT\" \"$KIVIO_HOOK_NAME\" \"$KIVIO_TOOL_NAME\" \"$KIVIO_TOOL_CALL_ID\" \"$KIVIO_CONVERSATION_ID\" > {env}",
            out = out.display(),
            env = env_out.display()
        ));
        let (dispatcher, failures) =
            dispatcher(&[hook], dir.path().to_path_buf()).expect("dispatcher");
        dispatcher.dispatch(
            HookEvent::AgentEnd,
            Some("write_file"),
            Some(2),
            Some("call-1"),
        );
        dispatcher.drain().await;

        let payload: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&out).expect("payload written"))
                .expect("payload is json");
        assert_eq!(payload["event"], "agent_end");
        assert_eq!(payload["hookName"], "probe");
        assert_eq!(payload["conversationId"], "conv-1");
        assert_eq!(payload["runId"], "run-1");
        assert_eq!(payload["messageId"], "msg-1");
        assert_eq!(payload["toolName"], "write_file");
        assert_eq!(payload["toolCallId"], "call-1");
        assert_eq!(payload["round"], 2);
        assert_eq!(payload["model"], "prov:model-x");

        assert_eq!(
            std::fs::read_to_string(&env_out).expect("env written"),
            "agent_end|probe|write_file|call-1|conv-1"
        );
        assert!(
            failures.try_recv().is_err(),
            "successful hook reports nothing"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn non_zero_exit_is_reported_without_breaking_the_run() {
        let dir = TempDir::new();
        let hook = command_hook("echo boom >&2; exit 3");
        let (dispatcher, failures) =
            dispatcher(&[hook], dir.path().to_path_buf()).expect("dispatcher");
        dispatcher.dispatch(HookEvent::AgentEnd, None, None, None);
        dispatcher.drain().await;

        let failure = failures.try_recv().expect("failure reported");
        assert_eq!(failure["event"], "agent_end");
        assert_eq!(failure["hookName"], "probe");
        assert_eq!(failure["conversationId"], "conv-1");
        let message = failure["message"].as_str().unwrap_or_default();
        assert!(message.contains("code 3"), "unexpected message: {message}");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn cancel_drops_queued_hooks_and_kills_the_running_script() {
        let dir = TempDir::new();
        let started = dir.path().join("started");
        let finished = dir.path().join("finished");
        let queued_marker = dir.path().join("queued");

        let mut slow = command_hook(&format!(
            "touch {started}; sleep 30; touch {finished}",
            started = started.display(),
            finished = finished.display()
        ));
        slow.timeout_ms = 60_000;
        let mut queued = command_hook(&format!("touch {}", queued_marker.display()));
        queued.name = "queued".to_string();
        queued.event = "turn_end".to_string();

        let (dispatcher, _failures) =
            dispatcher(&[slow, queued], dir.path().to_path_buf()).expect("dispatcher");
        dispatcher.dispatch(HookEvent::AgentEnd, None, None, None);
        dispatcher.dispatch(HookEvent::TurnEnd, None, None, None);

        // 等第一个脚本真正跑起来，再取消——否则测的是「还没开始」而非「被杀」。
        for _ in 0..200 {
            if started.exists() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        assert!(started.exists(), "slow hook never started");
        dispatcher.cancel();
        dispatcher.drain().await;

        assert!(!finished.exists(), "running script survived cancel");
        assert!(!queued_marker.exists(), "queued hook ran after cancel");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn agent_end_dispatched_after_cancel_still_runs() {
        // 取消只作废「此刻之前」入队的 job；Drop guard 之后派发的 agent_end 仍须触发。
        let dir = TempDir::new();
        let marker = dir.path().join("end");
        let hook = command_hook(&format!("touch {}", marker.display()));
        let (dispatcher, _failures) =
            dispatcher(&[hook], dir.path().to_path_buf()).expect("dispatcher");
        dispatcher.cancel();
        dispatcher.dispatch(HookEvent::AgentEnd, None, None, None);
        dispatcher.drain().await;
        assert!(marker.exists(), "agent_end was swallowed by cancel");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn missing_conversation_workdir_falls_back_to_temp() {
        // 会话工作目录是懒创建的：一个从没用过工具的会话，这个路径可能不存在。
        // 直接拿去当 current_dir 会让 spawn 以 ENOENT 失败（脚本根本没跑）。
        let dir = TempDir::new();
        let marker = dir.path().join("ran");
        let hook = command_hook(&format!("touch {}", marker.display()));
        let missing = dir.path().join("conv_never_created");
        let (dispatcher, failures) = dispatcher(&[hook], missing).expect("dispatcher");
        dispatcher.dispatch(HookEvent::AgentEnd, None, None, None);
        dispatcher.drain().await;

        assert!(
            marker.exists(),
            "hook never ran: spawn failed on missing cwd"
        );
        assert!(failures.try_recv().is_err(), "hook should not have failed");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn cancel_racing_the_spawn_does_not_leak_the_process() {
        // cancel 落在「worker 检查完世代」与「pid 登记完」之间时，那次 cancel 看到的是
        // 空槽位。若 spawn 后不复检世代，这个进程就没人杀 —— 一路活到 run 结束之后。
        let dir = TempDir::new();
        let finished = dir.path().join("finished");
        let mut slow = command_hook(&format!(
            "sleep 30; touch {finished}",
            finished = finished.display()
        ));
        slow.timeout_ms = 60_000;
        let (dispatcher, _failures) =
            dispatcher(&[slow], dir.path().to_path_buf()).expect("dispatcher");
        dispatcher.dispatch(HookEvent::AgentEnd, None, None, None);
        // 不等脚本起来就立刻取消，尽量落进 spawn 与 pid 登记之间的窗口。
        dispatcher.cancel();
        dispatcher.drain().await;
        assert!(
            !finished.exists(),
            "script outlived a cancel racing its spawn"
        );
    }

    /// 生产路径不是 `drain()`（那是单测特权），而是 **Drop**：`ChatAgentHost` 随
    /// `complete_assistant_reply_inner` 返回而落地，带走 dispatcher。这条验证
    /// 「dispatch(agent_end) 后立刻 drop，脚本仍然跑完」——之前所有测试都 drain，
    /// 等于从没测过真实的收尾路径。
    #[cfg(unix)]
    #[tokio::test]
    async fn agent_end_survives_drop_without_drain() {
        let dir = TempDir::new();
        let marker = dir.path().join("dropped");
        let hook = command_hook(&format!("touch {}", marker.display()));
        let (dispatcher, _failures) =
            dispatcher(&[hook], dir.path().to_path_buf()).expect("dispatcher");
        dispatcher.dispatch(HookEvent::AgentEnd, None, None, None);
        drop(dispatcher);
        // 给 detached worker 时间跑完（生产里 app 一直活着，这里模拟那段时间）。
        for _ in 0..100 {
            if marker.exists() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        assert!(
            marker.exists(),
            "agent_end was lost when the dispatcher dropped"
        );
    }

    fn http_hook(url: &str, method: &str) -> HookDef {
        HookDef {
            id: "h1".to_string(),
            name: "probe".to_string(),
            event: "agent_end".to_string(),
            enabled: true,
            kind: "http".to_string(),
            url: url.to_string(),
            method: method.to_string(),
            timeout_ms: 10_000,
            ..Default::default()
        }
    }

    /// 本地 HTTP/1.1 一次性应答。`delay` 在读完请求之后、写响应之前等待。
    /// `accepted` 在 accept 之后置位，给 cancel 测试一个「请求已经在飞」的边沿。
    async fn spawn_http_once(
        status_line: &'static str,
        delay: std::time::Duration,
        accepted: Option<std::sync::Arc<AtomicBool>>,
    ) -> String {
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind hook http fixture");
        let addr = listener.local_addr().expect("local addr");
        tokio::spawn(async move {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            if let Some(flag) = accepted {
                flag.store(true, Ordering::SeqCst);
            }
            let mut buf = vec![0u8; 4096];
            let _ = stream.read(&mut buf).await;
            if !delay.is_zero() {
                tokio::time::sleep(delay).await;
            }
            let body = b"ok";
            let resp = format!(
                "HTTP/1.1 {status_line}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(resp.as_bytes()).await;
            let _ = stream.write_all(body).await;
        });
        format!("http://{addr}/hook")
    }

    #[tokio::test]
    async fn http_2xx_is_success() {
        let url = spawn_http_once("200 OK", std::time::Duration::ZERO, None).await;
        let (dispatcher, failures) =
            dispatcher(&[http_hook(&url, "POST")], std::env::temp_dir()).expect("dispatcher");
        dispatcher.dispatch(HookEvent::AgentEnd, None, None, None);
        dispatcher.drain().await;
        assert!(
            failures.try_recv().is_err(),
            "2xx http hook should not report failure"
        );
    }

    #[tokio::test]
    async fn http_non_2xx_is_reported() {
        let url =
            spawn_http_once("500 Internal Server Error", std::time::Duration::ZERO, None).await;
        let (dispatcher, failures) =
            dispatcher(&[http_hook(&url, "POST")], std::env::temp_dir()).expect("dispatcher");
        dispatcher.dispatch(HookEvent::AgentEnd, None, None, None);
        dispatcher.drain().await;
        let failure = failures.try_recv().expect("failure reported");
        let message = failure["message"].as_str().unwrap_or_default();
        assert!(message.contains("500"), "unexpected message: {message}");
    }

    #[tokio::test]
    async fn http_redirect_is_not_success() {
        let url = spawn_http_once("302 Found", std::time::Duration::ZERO, None).await;
        let (dispatcher, failures) =
            dispatcher(&[http_hook(&url, "POST")], std::env::temp_dir()).expect("dispatcher");
        dispatcher.dispatch(HookEvent::AgentEnd, None, None, None);
        dispatcher.drain().await;
        let failure = failures.try_recv().expect("3xx must fail");
        let message = failure["message"].as_str().unwrap_or_default();
        assert!(message.contains("302"), "unexpected message: {message}");
    }

    #[tokio::test]
    async fn cancel_aborts_in_flight_http_without_reporting() {
        let accepted = std::sync::Arc::new(AtomicBool::new(false));
        let url = spawn_http_once(
            "200 OK",
            std::time::Duration::from_secs(30),
            Some(Arc::clone(&accepted)),
        )
        .await;
        let mut slow = http_hook(&url, "POST");
        slow.timeout_ms = 60_000;
        let (dispatcher, failures) = dispatcher(&[slow], std::env::temp_dir()).expect("dispatcher");
        dispatcher.dispatch(HookEvent::AgentEnd, None, None, None);
        for _ in 0..200 {
            if accepted.load(Ordering::SeqCst) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        assert!(accepted.load(Ordering::SeqCst), "http hook never started");
        dispatcher.cancel();
        tokio::time::timeout(std::time::Duration::from_secs(3), dispatcher.drain())
            .await
            .expect("cancel must abort the in-flight http hook");
        assert!(
            failures.try_recv().is_err(),
            "cancelled http hook must stay silent"
        );
    }

    #[tokio::test]
    async fn get_http_hook_omits_body() {
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        let saw = std::sync::Arc::new(Mutex::new(String::new()));
        let saw_clone = Arc::clone(&saw);
        tokio::spawn(async move {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            let mut buf = vec![0u8; 4096];
            let n = stream.read(&mut buf).await.unwrap_or(0);
            if let Ok(mut slot) = saw_clone.lock() {
                *slot = String::from_utf8_lossy(&buf[..n]).into_owned();
            }
            let resp = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok";
            let _ = stream.write_all(resp).await;
        });
        let url = format!("http://{addr}/hook");
        let (dispatcher, failures) =
            dispatcher(&[http_hook(&url, "GET")], std::env::temp_dir()).expect("dispatcher");
        dispatcher.dispatch(HookEvent::AgentEnd, None, None, None);
        dispatcher.drain().await;
        assert!(failures.try_recv().is_err(), "GET 200 should succeed");
        let request = saw.lock().map(|slot| slot.clone()).unwrap_or_default();
        assert!(
            request.starts_with("GET "),
            "expected GET, got: {request:?}"
        );
        assert!(
            !request.contains("\r\n\r\n{"),
            "GET must not carry a JSON body: {request:?}"
        );
    }
}
