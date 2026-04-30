// Apple Intelligence 客户端：以 Tauri sidecar 形式运行 Swift `keylingo-ai-helper`，
// 通过 stdin/stdout JSON 行协议把 Foundation Models 的 text/stream 调用桥接到 Rust。
//
// 协议见 src-tauri/swift/keylingo-ai-helper/Sources/main.swift。
// 单例：app 启动时 spawn 一次，所有请求复用同一个进程；按递增 id 路由响应到对应 channel。
// 不可用场景（Windows / 非 Apple Silicon / macOS 25 之前 / 用户没开 Apple Intelligence）：
//   sidecar 二进制不存在或 ready 报 unavailable → available=false，后续 call_* 直接 Err。

use std::collections::HashMap;
use std::sync::{
  atomic::{AtomicBool, AtomicU64, Ordering},
  Arc, Mutex,
};

use serde::Deserialize;
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
  Ready { available: bool, reason: Option<String> },
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
  next_id: AtomicU64,
  pending: Mutex<HashMap<u64, mpsc::UnboundedSender<RequestEvent>>>,
  // 写 stdin 必须 &mut self；用 Mutex 包裹 CommandChild 让多个 await 任务串行写
  child: Mutex<Option<CommandChild>>,
}

impl AppleIntelligenceClient {
  /// 不带 sidecar 的纯客户端实例：available=false，所有调用立即 Err。仅测试用。
  #[cfg(test)]
  pub fn disabled() -> Arc<Self> {
    Arc::new(Self {
      available: AtomicBool::new(false),
      next_id: AtomicU64::new(1),
      pending: Mutex::new(HashMap::new()),
      child: Mutex::new(None),
    })
  }

  pub fn new(app: &AppHandle) -> Arc<Self> {
    let me = Arc::new(Self {
      available: AtomicBool::new(false),
      next_id: AtomicU64::new(1),
      pending: Mutex::new(HashMap::new()),
      child: Mutex::new(None),
    });

    let sidecar = match app.shell().sidecar("keylingo-ai-helper") {
      Ok(c) => c,
      Err(err) => {
        eprintln!("[apple-intelligence] sidecar 不存在或未配置: {err}");
        return me;
      }
    };
    let (mut rx, child) = match sidecar.spawn() {
      Ok(pair) => pair,
      Err(err) => {
        eprintln!("[apple-intelligence] sidecar spawn 失败: {err}");
        return me;
      }
    };
    *me.child.lock().unwrap() = Some(child);

    let me_for_reader = me.clone();
    tauri::async_runtime::spawn(async move {
      while let Some(ev) = rx.recv().await {
        match ev {
          CommandEvent::Stdout(line) => {
            let s = String::from_utf8_lossy(&line);
            for piece in s.lines() {
              let trimmed = piece.trim();
              if trimmed.is_empty() { continue; }
              match serde_json::from_str::<SidecarEvent>(trimmed) {
                Ok(parsed) => me_for_reader.dispatch(parsed),
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
            me_for_reader.available.store(false, Ordering::SeqCst);
            // 把所有还在 await 的请求一并 Err 收尾,防止 caller 永远等不到响应
            let drained: Vec<mpsc::UnboundedSender<RequestEvent>> = {
              let mut guard = me_for_reader.pending.lock().unwrap();
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

    me
  }

  fn dispatch(&self, ev: SidecarEvent) {
    match ev {
      SidecarEvent::Ready { available, reason } => {
        self.available.store(available, Ordering::SeqCst);
        if available {
          println!("[apple-intelligence] ready: available=true");
        } else {
          println!("[apple-intelligence] ready: available=false reason={:?}", reason);
        }
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

  pub fn available(&self) -> bool {
    self.available.load(Ordering::SeqCst)
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
  pub async fn call_text(&self, prompt: &str) -> Result<String, String> {
    if !self.available() {
      return Err("Apple Intelligence 不可用".into());
    }
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
  pub async fn stream_text<F>(&self, prompt: &str, mut on_delta: F) -> Result<(), String>
  where
    F: FnMut(&str),
  {
    if !self.available() {
      return Err("Apple Intelligence 不可用".into());
    }
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

  /// Apple Vision 端上 OCR：把图像中的文字按行识别拼接返回。Vision 框架不依赖 FoundationModels,
  /// 所以即便 Foundation Models 不可用(available=false)、只要 sidecar 二进制能跑就行。
  /// 但当前代码仍然把 OCR 也门控在 available 后面 —— 因为 sidecar 是同一个进程,available=false 时它已经退出了。
  pub async fn ocr_image(&self, image_path: &str) -> Result<String, String> {
    if !self.available() {
      return Err("Apple Intelligence sidecar 不可用".into());
    }
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
