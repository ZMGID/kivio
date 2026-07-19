#[cfg(target_os = "macos")]
use std::collections::HashMap;
#[cfg(target_os = "macos")]
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc, Mutex,
};

#[cfg(target_os = "macos")]
use serde::Deserialize;
#[cfg(target_os = "macos")]
use tauri::async_runtime::JoinHandle;
#[cfg(target_os = "macos")]
use tauri::async_runtime::Receiver;
#[cfg(target_os = "macos")]
use tauri::AppHandle;
#[cfg(target_os = "macos")]
use tauri_plugin_shell::process::{CommandChild, CommandEvent};
#[cfg(target_os = "macos")]
use tauri_plugin_shell::ShellExt;
#[cfg(target_os = "macos")]
use tokio::sync::mpsc;

#[cfg(target_os = "macos")]
#[derive(Deserialize, Debug)]
#[serde(tag = "type")]
enum OcrSidecarEvent {
    #[serde(rename = "ready")]
    Ready { available: bool },
    #[serde(rename = "done")]
    Done { id: u64, content: Option<String> },
    #[serde(rename = "error")]
    Error { id: u64, message: String },
}

#[cfg(target_os = "macos")]
#[derive(Debug)]
enum OcrRequestEvent {
    Done(Option<String>),
    Error(String),
}

#[cfg(target_os = "macos")]
pub struct MacOcrClient {
    available: AtomicBool,
    permanently_unavailable: AtomicBool,
    shutting_down: AtomicBool,
    next_id: AtomicU64,
    idle_generation: AtomicU64,
    idle_shutdown_task: Mutex<Option<JoinHandle<()>>>,
    pending: Mutex<HashMap<u64, mpsc::UnboundedSender<OcrRequestEvent>>>,
    child: Mutex<Option<CommandChild>>,
    app: Option<AppHandle>,
}

#[cfg(target_os = "macos")]
impl MacOcrClient {
    #[cfg(test)]
    pub fn disabled() -> Arc<Self> {
        Self::unavailable(None)
    }

    #[cfg(test)]
    fn unavailable(app: Option<AppHandle>) -> Arc<Self> {
        Arc::new(Self {
            available: AtomicBool::new(false),
            permanently_unavailable: AtomicBool::new(true),
            shutting_down: AtomicBool::new(true),
            next_id: AtomicU64::new(1),
            idle_generation: AtomicU64::new(0),
            idle_shutdown_task: Mutex::new(None),
            pending: Mutex::new(HashMap::new()),
            child: Mutex::new(None),
            app,
        })
    }

    pub fn new(app: &AppHandle) -> Arc<Self> {
        Arc::new(Self {
            available: AtomicBool::new(false),
            permanently_unavailable: AtomicBool::new(false),
            shutting_down: AtomicBool::new(false),
            next_id: AtomicU64::new(1),
            idle_generation: AtomicU64::new(0),
            idle_shutdown_task: Mutex::new(None),
            pending: Mutex::new(HashMap::new()),
            child: Mutex::new(None),
            app: Some(app.clone()),
        })
    }

    /// Headless (no-AppHandle) client for the `kivio-code` CLI, which has no
    /// Tauri runtime. OCR is never invoked from the terminal agent, so this
    /// mirrors `new` but carries `app: None` (the sidecar can never be spawned).
    pub fn headless() -> Arc<Self> {
        Arc::new(Self {
            available: AtomicBool::new(false),
            permanently_unavailable: AtomicBool::new(true),
            shutting_down: AtomicBool::new(true),
            next_id: AtomicU64::new(1),
            idle_generation: AtomicU64::new(0),
            idle_shutdown_task: Mutex::new(None),
            pending: Mutex::new(HashMap::new()),
            child: Mutex::new(None),
            app: None,
        })
    }

    fn dispatch(&self, ev: OcrSidecarEvent) {
        match ev {
            OcrSidecarEvent::Ready { available } => {
                self.available.store(available, Ordering::SeqCst);
                if !available {
                    self.permanently_unavailable.store(true, Ordering::SeqCst);
                }
            }
            OcrSidecarEvent::Done { id, content } => {
                let sender = self.pending.lock().unwrap().remove(&id);
                if let Some(s) = sender {
                    let _ = s.send(OcrRequestEvent::Done(content));
                }
            }
            OcrSidecarEvent::Error { id, message } => {
                let sender = self.pending.lock().unwrap().remove(&id);
                if let Some(s) = sender {
                    let _ = s.send(OcrRequestEvent::Error(message));
                }
            }
        }
    }

    fn spawn_reader_task(me: Arc<Self>, pid: u32, mut rx: Receiver<CommandEvent>) {
        tauri::async_runtime::spawn(async move {
            while let Some(ev) = rx.recv().await {
                match ev {
                    CommandEvent::Stdout(line) => {
                        let s = String::from_utf8_lossy(&line);
                        for piece in s.lines() {
                            let trimmed = piece.trim();
                            if trimmed.is_empty() {
                                continue;
                            }
                            match serde_json::from_str::<OcrSidecarEvent>(trimmed) {
                                Ok(parsed) => me.dispatch(parsed),
                                Err(e) => eprintln!("[macos-ocr] parse 失败: {e} line={trimmed}"),
                            }
                        }
                    }
                    CommandEvent::Stderr(line) => {
                        eprintln!("[macos-ocr] stderr: {}", String::from_utf8_lossy(&line));
                    }
                    CommandEvent::Error(err) => {
                        eprintln!("[macos-ocr] sidecar error: {err}");
                    }
                    CommandEvent::Terminated(payload) => {
                        eprintln!("[macos-ocr] sidecar terminated: {:?}", payload);
                        let was_current = {
                            let mut child = me.child.lock().unwrap();
                            if child.as_ref().is_some_and(|child| child.pid() == pid) {
                                *child = None;
                                true
                            } else {
                                false
                            }
                        };
                        // A previous idle helper may terminate after a replacement
                        // was already spawned. Never clear/drain the replacement.
                        if was_current {
                            me.available.store(false, Ordering::SeqCst);
                            me.fail_all_pending("OCR helper 进程已退出");
                        }
                        break;
                    }
                    _ => {}
                }
            }
        });
    }

    fn ensure_started(self: &Arc<Self>) -> Result<(), String> {
        if self.shutting_down.load(Ordering::SeqCst) {
            return Err("OCR helper 已关闭".into());
        }
        let mut guard = self.child.lock().unwrap();
        // Close the check-vs-lock race with shutdown(): once shutdown owns and
        // clears the child slot, no waiter that observed the old state may spawn
        // a replacement afterward.
        if self.shutting_down.load(Ordering::SeqCst) {
            return Err("OCR helper 已关闭".into());
        }
        if guard.is_some() {
            return Ok(());
        }
        if self.permanently_unavailable.load(Ordering::SeqCst) {
            return Err("macOS OCR 不可用".into());
        }

        let Some(app) = self.app.as_ref() else {
            self.permanently_unavailable.store(true, Ordering::SeqCst);
            return Err("macOS OCR 不可用".into());
        };

        let sidecar = app.shell().sidecar("kivio-ocr-helper").map_err(|err| {
            self.permanently_unavailable.store(true, Ordering::SeqCst);
            eprintln!("[macos-ocr] sidecar 不存在或未配置: {err}");
            "macOS OCR 不可用".to_string()
        })?;

        let (rx, child) = sidecar.spawn().map_err(|err| {
            eprintln!("[macos-ocr] sidecar spawn 失败: {err}");
            "macOS OCR 启动失败".to_string()
        })?;
        let pid = child.pid();
        *guard = Some(child);
        drop(guard);

        Self::spawn_reader_task(self.clone(), pid, rx);
        Ok(())
    }

    fn write_line(&self, line: String) -> Result<(), String> {
        let mut guard = self.child.lock().unwrap();
        let child = guard
            .as_mut()
            .ok_or_else(|| "OCR helper 未启动".to_string())?;
        child
            .write(line.as_bytes())
            .map_err(|e| format!("写 OCR helper stdin 失败: {e}"))
    }

    fn register(&self, id: u64) -> mpsc::UnboundedReceiver<OcrRequestEvent> {
        let (tx, rx) = mpsc::unbounded_channel();
        self.pending.lock().unwrap().insert(id, tx);
        rx
    }

    fn fail_all_pending(&self, message: &str) {
        let drained: Vec<mpsc::UnboundedSender<OcrRequestEvent>> = {
            let mut guard = self.pending.lock().unwrap();
            guard.drain().map(|(_, sender)| sender).collect()
        };
        for sender in drained {
            let _ = sender.send(OcrRequestEvent::Error(message.to_string()));
        }
    }

    fn stop_child(&self, force: bool) {
        let child = self.child.lock().unwrap().take();
        if let Some(mut child) = child {
            // Graceful protocol shutdown closes Vision/Swift state cleanly. On
            // app exit, kill is still used as a bounded fallback.
            let _ = child.write(b"{\"action\":\"shutdown\"}\n");
            if force {
                let _ = child.kill();
            }
        }
        self.available.store(false, Ordering::SeqCst);
    }

    fn schedule_idle_shutdown(self: &Arc<Self>) {
        const OCR_HELPER_IDLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

        if self.shutting_down.load(Ordering::SeqCst) {
            return;
        }
        if let Some(task) = self.idle_shutdown_task.lock().unwrap().take() {
            task.abort();
        }
        let generation = self.idle_generation.fetch_add(1, Ordering::SeqCst) + 1;
        let me = self.clone();
        let task = tauri::async_runtime::spawn(async move {
            tokio::time::sleep(OCR_HELPER_IDLE_TIMEOUT).await;
            if me.idle_generation.load(Ordering::SeqCst) != generation {
                return;
            }
            if !me.pending.lock().unwrap().is_empty() {
                return;
            }
            me.stop_child(false);
        });
        *self.idle_shutdown_task.lock().unwrap() = Some(task);
    }

    pub fn shutdown(&self) {
        self.shutting_down.store(true, Ordering::SeqCst);
        self.idle_generation.fetch_add(1, Ordering::SeqCst);
        if let Some(task) = self.idle_shutdown_task.lock().unwrap().take() {
            task.abort();
        }
        self.fail_all_pending("OCR helper 已关闭");
        self.stop_child(true);
    }

    pub async fn ocr_image(self: &Arc<Self>, image_path: &str) -> Result<String, String> {
        if self.shutting_down.load(Ordering::SeqCst) {
            return Err("OCR helper 已关闭".into());
        }
        self.idle_generation.fetch_add(1, Ordering::SeqCst);
        if let Some(task) = self.idle_shutdown_task.lock().unwrap().take() {
            task.abort();
        }
        self.ensure_started()?;
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let mut rx = self.register(id);
        let body = serde_json::json!({ "id": id, "action": "ocr", "imagePath": image_path });
        if self.write_line(format!("{body}\n")).is_err() {
            // The helper may have reached its idle exit just before this write.
            // Every request already sent to that PID is now indeterminate. Fail
            // them together before replacing the process; otherwise the old
            // reader's PID isolation would correctly ignore its late termination
            // event, but those request receivers would remain pending forever.
            self.fail_all_pending("OCR helper 管道已断开，进程正在重启");
            self.stop_child(true);
            if let Err(err) = self.ensure_started() {
                return Err(err);
            }
            rx = self.register(id);
            if let Err(err) = self.write_line(format!("{body}\n")) {
                self.pending.lock().unwrap().remove(&id);
                self.stop_child(true);
                return Err(err);
            }
        }
        while let Some(ev) = rx.recv().await {
            match ev {
                OcrRequestEvent::Done(content) => {
                    self.schedule_idle_shutdown();
                    return Ok(content.unwrap_or_default());
                }
                OcrRequestEvent::Error(msg) => {
                    self.schedule_idle_shutdown();
                    return Err(msg);
                }
            }
        }
        self.schedule_idle_shutdown();
        Err("OCR helper 通道意外关闭".into())
    }
}

#[cfg(target_os = "macos")]
impl Drop for MacOcrClient {
    fn drop(&mut self) {
        if let Ok(task) = self.idle_shutdown_task.get_mut() {
            if let Some(task) = task.take() {
                task.abort();
            }
        }
        if let Ok(child) = self.child.get_mut() {
            if let Some(mut child) = child.take() {
                let _ = child.write(b"{\"action\":\"shutdown\"}\n");
                let _ = child.kill();
            }
        }
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[tokio::test]
    async fn fail_all_pending_notifies_every_receiver_and_clears_registry() {
        let client = MacOcrClient::unavailable(None);
        let mut first = client.register(1);
        let mut second = client.register(2);

        client.fail_all_pending("helper restarted");

        assert!(matches!(
            first.recv().await,
            Some(OcrRequestEvent::Error(message)) if message == "helper restarted"
        ));
        assert!(matches!(
            second.recv().await,
            Some(OcrRequestEvent::Error(message)) if message == "helper restarted"
        ));
        assert!(client.pending.lock().unwrap().is_empty());
    }

    #[test]
    fn shutdown_is_idempotent_and_cannot_reschedule_idle_cleanup() {
        let client = MacOcrClient::unavailable(None);
        client.shutting_down.store(false, Ordering::SeqCst);

        client.shutdown();
        client.shutdown();
        client.schedule_idle_shutdown();

        assert!(client.shutting_down.load(Ordering::SeqCst));
        assert!(client.idle_shutdown_task.lock().unwrap().is_none());
        assert!(client.child.lock().unwrap().is_none());
    }
}
