// Apple Intelligence 客户端：以 Tauri sidecar 形式运行 Swift `kivio-ai-helper`，
// 通过 stdin/stdout JSON 行协议把 Foundation Models 的 text/stream 调用桥接到 Rust。
//
// 协议见 src-tauri/swift/kivio-ai-helper/Sources/main.swift。
// 单例：首次真正用到 Apple 路由时才 spawn，一旦启动后所有请求复用同一个进程；
// 按递增 id 路由响应到对应 channel。
// 不可用场景（Windows / 非 Apple Silicon / macOS 25 之前 / 用户没开 Apple Intelligence）：
//   sidecar 二进制不存在或 ready 报 unavailable → available=false，后续 call_* 直接 Err。

use std::collections::HashMap;
use std::sync::{
  atomic::{AtomicBool, AtomicU64, Ordering},
  Arc, Mutex,
};

use serde::Deserialize;
use tauri::async_runtime::Receiver;
use tauri::AppHandle;
use tauri_plugin_shell::process::{CommandChild, CommandEvent};
use tauri_plugin_shell::ShellExt;
use tokio::sync::mpsc;

/// provider.base_url 的哨兵值。route 各 OpenAI 调用顶部 check 这个值即可绕道到 sidecar。
pub const APPLE_INTELLIGENCE_BASE_URL: &str = "applefoundation://local";

#[derive(Deserialize, Debug)]
#[serde(tag = "type")]
enum SidecarEvent {
  #[serde(rename = "ready")]
  Ready { available: bool },
  #[serde(rename = "chunk")]
  Chunk { id: u64, delta: String },
  #[serde(rename = "done")]
  Done { id: u64, content: Option<String> },
  #[serde(rename = "error")]
  Error { id: u64, message: String },
}

#[derive(Debug)]
enum RequestEvent {
  Chunk(String),
  Done(Option<String>),
  Error(String),
}

pub struct AppleIntelligenceClient {
  available: AtomicBool,
  /// sidecar 是否已确认不可用（主要是二进制缺失/未配置）。
  /// 一旦置 true，后续不再反复尝试拉起，避免每次请求都重复 spawn 失败。
  permanently_unavailable: AtomicBool,
  next_id: AtomicU64,
  pending: Mutex<HashMap<u64, mpsc::UnboundedSender<RequestEvent>>>,
  // 写 stdin 必须 &mut self；用 Mutex 包裹 CommandChild 让多个 await 任务串行写
  child: Mutex<Option<CommandChild>>,
  app: Option<AppHandle>,
}

impl AppleIntelligenceClient {
  /// 不带 sidecar 的纯客户端实例：available=false，所有调用立即 Err。仅测试用。
  #[cfg(test)]
  pub fn disabled() -> Arc<Self> {
    Arc::new(Self {
      available: AtomicBool::new(false),
      permanently_unavailable: AtomicBool::new(true),
      next_id: AtomicU64::new(1),
      pending: Mutex::new(HashMap::new()),
      child: Mutex::new(None),
      app: None,
    })
  }

  pub fn new(app: &AppHandle) -> Arc<Self> {
    Arc::new(Self {
      available: AtomicBool::new(false),
      permanently_unavailable: AtomicBool::new(false),
      next_id: AtomicU64::new(1),
      pending: Mutex::new(HashMap::new()),
      child: Mutex::new(None),
      app: Some(app.clone()),
    })
  }

  fn dispatch(&self, ev: SidecarEvent) {
    match ev {
      SidecarEvent::Ready { available, .. } => {
        self.available.store(available, Ordering::SeqCst);
      }
      SidecarEvent::Chunk { id, delta } => {
        let sender = self.pending.lock().unwrap().get(&id).cloned();
        if let Some(s) = sender {
          let _ = s.send(RequestEvent::Chunk(delta));
        }
      }
      SidecarEvent::Done { id, content } => {
        let sender = self.pending.lock().unwrap().remove(&id);
        if let Some(s) = sender {
          let _ = s.send(RequestEvent::Done(content));
        }
      }
      SidecarEvent::Error { id, message } => {
        let sender = self.pending.lock().unwrap().remove(&id);
        if let Some(s) = sender {
          let _ = s.send(RequestEvent::Error(message));
        }
      }
    }
  }

  pub fn app_handle(&self) -> Option<&AppHandle> {
    self.app.as_ref()
  }

  fn spawn_reader_task(me: Arc<Self>, mut rx: Receiver<CommandEvent>) {
    tauri::async_runtime::spawn(async move {
      while let Some(ev) = rx.recv().await {
        match ev {
          CommandEvent::Stdout(line) => {
            let s = String::from_utf8_lossy(&line);
            for piece in s.lines() {
              let trimmed = piece.trim();
              if trimmed.is_empty() { continue; }
              match serde_json::from_str::<SidecarEvent>(trimmed) {
                Ok(parsed) => me.dispatch(parsed),
                Err(e) => eprintln!("[apple-intelligence] parse 失败: {e} line={trimmed}"),
              }
            }
          }
          CommandEvent::Stderr(line) => {
            eprintln!("[apple-intelligence] stderr: {}", String::from_utf8_lossy(&line));
          }
          CommandEvent::Error(err) => {
            eprintln!("[apple-intelligence] sidecar error: {err}");
          }
          CommandEvent::Terminated(payload) => {
            eprintln!("[apple-intelligence] sidecar terminated: {:?}", payload);
            me.available.store(false, Ordering::SeqCst);
            {
              let mut child = me.child.lock().unwrap();
              *child = None;
            }
            // 把所有还在 await 的请求一并 Err 收尾,防止 caller 永远等不到响应
            let drained: Vec<mpsc::UnboundedSender<RequestEvent>> = {
              let mut guard = me.pending.lock().unwrap();
              guard.drain().map(|(_, sender)| sender).collect()
            };
            for sender in drained {
              let _ = sender.send(RequestEvent::Error("sidecar 进程已退出".into()));
            }
            break;
          }
          _ => {}
        }
      }
    });
  }

  fn ensure_started(self: &Arc<Self>) -> Result<(), String> {
    let mut guard = self.child.lock().unwrap();
    if guard.is_some() { return Ok(()); }
    if self.permanently_unavailable.load(Ordering::SeqCst) {
      return Err("Apple Intelligence 不可用".into());
    }

    let Some(app) = self.app.as_ref() else {
      self.permanently_unavailable.store(true, Ordering::SeqCst);
      return Err("Apple Intelligence 不可用".into());
    };

    let sidecar = app
      .shell()
      .sidecar("kivio-ai-helper")
      .map_err(|err| {
        self.permanently_unavailable.store(true, Ordering::SeqCst);
        eprintln!("[apple-intelligence] sidecar 不存在或未配置: {err}");
        "Apple Intelligence 不可用".to_string()
      })?;

    let (rx, child) = sidecar.spawn().map_err(|err| {
      eprintln!("[apple-intelligence] sidecar spawn 失败: {err}");
      "Apple Intelligence 启动失败".to_string()
    })?;
    *guard = Some(child);
    drop(guard);

    Self::spawn_reader_task(self.clone(), rx);
    Ok(())
  }

  fn write_line(&self, line: String) -> Result<(), String> {
    let mut guard = self.child.lock().unwrap();
    let child = guard.as_mut().ok_or_else(|| "sidecar 未启动".to_string())?;
    child.write(line.as_bytes()).map_err(|e| format!("写 stdin 失败: {e}"))
  }

  fn register(&self, id: u64) -> mpsc::UnboundedReceiver<RequestEvent> {
    let (tx, rx) = mpsc::unbounded_channel();
    self.pending.lock().unwrap().insert(id, tx);
    rx
  }

  /// 一次性返回完整内容
  pub async fn call_text(self: &Arc<Self>, prompt: &str) -> Result<String, String> {
    self.ensure_started()?;
    let id = self.next_id.fetch_add(1, Ordering::SeqCst);
    let mut rx = self.register(id);
    let body = serde_json::json!({ "id": id, "action": "text", "prompt": prompt });
    self.write_line(format!("{body}\n"))?;
    while let Some(ev) = rx.recv().await {
      match ev {
        RequestEvent::Done(content) => return Ok(content.unwrap_or_default().trim().to_string()),
        RequestEvent::Error(msg) => return Err(msg),
        RequestEvent::Chunk(_) => {} // text 模式不应该产 chunk，忽略
      }
    }
    Err("sidecar 通道意外关闭".into())
  }

  /// 流式输出，每个 delta 调用一次 on_delta
  pub async fn stream_text<F>(self: &Arc<Self>, prompt: &str, mut on_delta: F) -> Result<(), String>
  where
    F: FnMut(&str),
  {
    self.ensure_started()?;
    let id = self.next_id.fetch_add(1, Ordering::SeqCst);
    let mut rx = self.register(id);
    let body = serde_json::json!({ "id": id, "action": "stream", "prompt": prompt });
    self.write_line(format!("{body}\n"))?;
    while let Some(ev) = rx.recv().await {
      match ev {
        RequestEvent::Chunk(delta) => on_delta(&delta),
        RequestEvent::Done(_) => return Ok(()),
        RequestEvent::Error(msg) => return Err(msg),
      }
    }
    Err("sidecar 通道意外关闭".into())
  }

  /// Apple Vision 端上 OCR：把图像中的文字按行识别拼接返回。Vision 框架不依赖 FoundationModels，
  /// 首次调用时 ensure_started() 自动拉起 sidecar；sidecar 缺失或 spawn 失败时返回 Err。
  pub async fn ocr_image(self: &Arc<Self>, image_path: &str) -> Result<String, String> {
    self.ensure_started()?;
    let id = self.next_id.fetch_add(1, Ordering::SeqCst);
    let mut rx = self.register(id);
    let body = serde_json::json!({ "id": id, "action": "ocr", "imagePath": image_path });
    self.write_line(format!("{body}\n"))?;
    while let Some(ev) = rx.recv().await {
      match ev {
        RequestEvent::Done(content) => return Ok(content.unwrap_or_default()),
        RequestEvent::Error(msg) => return Err(msg),
        RequestEvent::Chunk(_) => {}
      }
    }
    Err("sidecar 通道意外关闭".into())
  }
}
