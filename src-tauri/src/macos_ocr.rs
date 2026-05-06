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
  next_id: AtomicU64,
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
      next_id: AtomicU64::new(1),
      pending: Mutex::new(HashMap::new()),
      child: Mutex::new(None),
      app,
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

  fn spawn_reader_task(me: Arc<Self>, mut rx: Receiver<CommandEvent>) {
    tauri::async_runtime::spawn(async move {
      while let Some(ev) = rx.recv().await {
        match ev {
          CommandEvent::Stdout(line) => {
            let s = String::from_utf8_lossy(&line);
            for piece in s.lines() {
              let trimmed = piece.trim();
              if trimmed.is_empty() { continue; }
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
            me.available.store(false, Ordering::SeqCst);
            {
              let mut child = me.child.lock().unwrap();
              *child = None;
            }
            let drained: Vec<mpsc::UnboundedSender<OcrRequestEvent>> = {
              let mut guard = me.pending.lock().unwrap();
              guard.drain().map(|(_, sender)| sender).collect()
            };
            for sender in drained {
              let _ = sender.send(OcrRequestEvent::Error("OCR helper 进程已退出".into()));
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
      return Err("macOS OCR 不可用".into());
    }

    let Some(app) = self.app.as_ref() else {
      self.permanently_unavailable.store(true, Ordering::SeqCst);
      return Err("macOS OCR 不可用".into());
    };

    let sidecar = app
      .shell()
      .sidecar("kivio-ocr-helper")
      .map_err(|err| {
        self.permanently_unavailable.store(true, Ordering::SeqCst);
        eprintln!("[macos-ocr] sidecar 不存在或未配置: {err}");
        "macOS OCR 不可用".to_string()
      })?;

    let (rx, child) = sidecar.spawn().map_err(|err| {
      eprintln!("[macos-ocr] sidecar spawn 失败: {err}");
      "macOS OCR 启动失败".to_string()
    })?;
    *guard = Some(child);
    drop(guard);

    Self::spawn_reader_task(self.clone(), rx);
    Ok(())
  }

  fn write_line(&self, line: String) -> Result<(), String> {
    let mut guard = self.child.lock().unwrap();
    let child = guard.as_mut().ok_or_else(|| "OCR helper 未启动".to_string())?;
    child.write(line.as_bytes()).map_err(|e| format!("写 OCR helper stdin 失败: {e}"))
  }

  fn register(&self, id: u64) -> mpsc::UnboundedReceiver<OcrRequestEvent> {
    let (tx, rx) = mpsc::unbounded_channel();
    self.pending.lock().unwrap().insert(id, tx);
    rx
  }

  pub async fn ocr_image(self: &Arc<Self>, image_path: &str) -> Result<String, String> {
    self.ensure_started()?;
    let id = self.next_id.fetch_add(1, Ordering::SeqCst);
    let mut rx = self.register(id);
    let body = serde_json::json!({ "id": id, "action": "ocr", "imagePath": image_path });
    self.write_line(format!("{body}\n"))?;
    while let Some(ev) = rx.recv().await {
      match ev {
        OcrRequestEvent::Done(content) => return Ok(content.unwrap_or_default()),
        OcrRequestEvent::Error(msg) => return Err(msg),
      }
    }
    Err("OCR helper 通道意外关闭".into())
  }
}
