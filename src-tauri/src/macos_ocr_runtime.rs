//! Apple Vision helper protocol, independent of the desktop window framework.
//! The worker is lazy, shared across requests, and released after two idle minutes.
use serde::Deserialize;
use std::{
    collections::HashMap,
    path::PathBuf,
    process::Stdio,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::Command,
    sync::{mpsc, oneshot},
    task::JoinHandle,
};

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum Event {
    Ready { available: bool },
    Done { id: u64, content: Option<String> },
    Error { id: u64, message: String },
}
enum WorkerCommand {
    Request(String),
    Stop { force: bool },
}
struct Worker {
    generation: u64,
    sender: mpsc::UnboundedSender<WorkerCommand>,
}
struct Pending {
    generation: u64,
    sender: oneshot::Sender<Result<String, String>>,
}
struct State {
    worker: Option<Worker>,
    pending: HashMap<u64, Pending>,
}

pub struct MacOcrClient {
    executable: Option<PathBuf>,
    state: Mutex<State>,
    shutting_down: AtomicBool,
    unavailable: AtomicBool,
    next_id: AtomicU64,
    worker_generation: AtomicU64,
    idle_generation: AtomicU64,
    idle_task: Mutex<Option<JoinHandle<()>>>,
}
impl MacOcrClient {
    pub fn new(executable: PathBuf) -> Arc<Self> {
        Self::create(Some(executable))
    }
    pub fn headless() -> Arc<Self> {
        Self::create(None)
    }
    fn create(executable: Option<PathBuf>) -> Arc<Self> {
        Arc::new(Self {
            unavailable: AtomicBool::new(executable.is_none()),
            executable,
            state: Mutex::new(State {
                worker: None,
                pending: HashMap::new(),
            }),
            shutting_down: AtomicBool::new(false),
            next_id: AtomicU64::new(1),
            worker_generation: AtomicU64::new(1),
            idle_generation: AtomicU64::new(0),
            idle_task: Mutex::new(None),
        })
    }
    pub fn bundled_helper() -> Result<PathBuf, String> {
        let executable = std::env::current_exe().map_err(|e| e.to_string())?;
        Ok(executable
            .parent()
            .ok_or("无法定位 OCR helper 目录")?
            .join("kivio-ocr-helper"))
    }
    fn start(self: &Arc<Self>, state: &mut State) -> Result<(), String> {
        if self.shutting_down.load(Ordering::SeqCst) {
            return Err("OCR helper 已关闭".into());
        }
        if self.unavailable.load(Ordering::SeqCst) {
            return Err("macOS OCR 不可用".into());
        }
        if state.worker.is_some() {
            return Ok(());
        }
        let path = self.executable.as_ref().ok_or("macOS OCR 不可用")?;
        let mut child = Command::new(path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| format!("macOS OCR 启动失败：{e}"))?;
        let mut stdin = child.stdin.take().ok_or("OCR helper stdin 不可用")?;
        let mut stdout =
            BufReader::new(child.stdout.take().ok_or("OCR helper stdout 不可用")?).lines();
        if let Some(stderr) = child.stderr.take() {
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    eprintln!("[macos-ocr] {line}");
                }
            });
        }
        let generation = self.worker_generation.fetch_add(1, Ordering::SeqCst);
        let (sender, mut receiver) = mpsc::unbounded_channel();
        state.worker = Some(Worker { generation, sender });
        let owner = Arc::downgrade(self);
        tokio::spawn(async move {
            let reason = loop {
                tokio::select! {
                    line = stdout.next_line() => match line {
                        Ok(Some(line)) => {
                            if let Ok(event) = serde_json::from_str::<Event>(&line) {
                                if let Some(owner) = owner.upgrade() { owner.dispatch(generation, event); }
                            }
                        }
                        Ok(None) => break "OCR helper 进程已退出".to_owned(),
                        Err(error) => break format!("读取 OCR helper 失败：{error}"),
                    },
                    command = receiver.recv() => match command {
                        Some(WorkerCommand::Request(line)) => {
                            if let Err(error) = stdin.write_all(line.as_bytes()).await { break format!("写 OCR helper stdin 失败：{error}"); }
                        }
                        Some(WorkerCommand::Stop { force }) => {
                            if !force {
                                let _ = stdin.write_all(b"{\"action\":\"shutdown\"}\n").await;
                                if tokio::time::timeout(Duration::from_secs(1), child.wait()).await.is_ok() { break "OCR helper 已关闭".to_owned(); }
                            }
                            let _ = child.kill().await;
                            break "OCR helper 已关闭".to_owned();
                        }
                        None => { let _ = child.kill().await; break "OCR helper 已关闭".to_owned(); }
                    },
                    _ = child.wait() => break "OCR helper 进程已退出".to_owned(),
                }
            };
            // Reap the process even when stdout closed before the child exited.
            let _ = child.kill().await;
            if let Some(owner) = owner.upgrade() {
                owner.worker_exited(generation, &reason);
            }
        });
        Ok(())
    }
    fn dispatch(&self, generation: u64, event: Event) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if !state
            .worker
            .as_ref()
            .is_some_and(|worker| worker.generation == generation)
        {
            return;
        }
        match event {
            Event::Ready { available: false } => {
                self.unavailable.store(true, Ordering::SeqCst);
                for (_, pending) in state.pending.drain() {
                    let _ = pending.sender.send(Err("macOS OCR 不可用".into()));
                }
            }
            Event::Ready { .. } => {}
            Event::Done { id, content } => {
                if let Some(pending) = state.pending.remove(&id) {
                    let _ = pending.sender.send(Ok(content.unwrap_or_default()));
                }
            }
            Event::Error { id, message } => {
                if let Some(pending) = state.pending.remove(&id) {
                    let _ = pending.sender.send(Err(message));
                }
            }
        }
    }
    fn worker_exited(&self, generation: u64, reason: &str) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state
            .worker
            .as_ref()
            .is_some_and(|worker| worker.generation == generation)
        {
            state.worker = None;
        }
        let ids: Vec<_> = state
            .pending
            .iter()
            .filter(|(_, pending)| pending.generation == generation)
            .map(|(id, _)| *id)
            .collect();
        for id in ids {
            if let Some(pending) = state.pending.remove(&id) {
                let _ = pending.sender.send(Err(reason.into()));
            }
        }
    }
    fn schedule_idle(self: &Arc<Self>) {
        if self.shutting_down.load(Ordering::SeqCst) {
            return;
        }
        let generation = self.idle_generation.fetch_add(1, Ordering::SeqCst) + 1;
        let owner = Arc::downgrade(self);
        let task = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(120)).await;
            if let Some(owner) = owner.upgrade() {
                let mut state = owner.state.lock().unwrap_or_else(|e| e.into_inner());
                if owner.idle_generation.load(Ordering::SeqCst) == generation
                    && state.pending.is_empty()
                {
                    if let Some(worker) = state.worker.take() {
                        let _ = worker.sender.send(WorkerCommand::Stop { force: false });
                    }
                }
            }
        });
        let mut slot = self.idle_task.lock().unwrap_or_else(|e| e.into_inner());
        if self.shutting_down.load(Ordering::SeqCst) {
            task.abort();
        } else if let Some(old) = slot.replace(task) {
            old.abort();
        }
    }
    pub async fn ocr_image(self: &Arc<Self>, image_path: &str) -> Result<String, String> {
        self.idle_generation.fetch_add(1, Ordering::SeqCst);
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let receiver = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            self.start(&mut state)?;
            let worker = state.worker.as_ref().unwrap();
            let generation = worker.generation;
            let line = format!(
                "{}\n",
                serde_json::json!({"id":id,"action":"ocr","imagePath":image_path})
            );
            worker
                .sender
                .send(WorkerCommand::Request(line))
                .map_err(|_| "OCR helper 管道已断开".to_string())?;
            let (sender, receiver) = oneshot::channel();
            state.pending.insert(id, Pending { generation, sender });
            receiver
        };
        let _request = RequestGuard {
            owner: Arc::downgrade(self),
            id,
        };
        receiver
            .await
            .map_err(|_| "OCR helper 通道意外关闭".to_string())?
    }
    pub fn shutdown(&self) {
        self.shutting_down.store(true, Ordering::SeqCst);
        self.idle_generation.fetch_add(1, Ordering::SeqCst);
        if let Some(task) = self
            .idle_task
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            task.abort();
        }
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(worker) = state.worker.take() {
            let _ = worker.sender.send(WorkerCommand::Stop { force: true });
        }
        for (_, pending) in state.pending.drain() {
            let _ = pending.sender.send(Err("OCR helper 已关闭".into()));
        }
    }
}
struct RequestGuard {
    owner: Weak<MacOcrClient>,
    id: u64,
}
impl Drop for RequestGuard {
    fn drop(&mut self) {
        if let Some(owner) = self.owner.upgrade() {
            owner
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .pending
                .remove(&self.id);
            owner.schedule_idle();
        }
    }
}
impl Drop for MacOcrClient {
    fn drop(&mut self) {
        self.shutdown();
    }
}
