//! Zeron's retained terminal tabs backed by Kivio's PTY owner.
use super::*;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use kivio_model_runtime::terminal::{self, TerminalEvent, TerminalService};
use std::{
    collections::{HashMap, VecDeque},
    sync::Mutex,
};

const REPLAY_BYTES: usize = 2 * 1024 * 1024;
#[derive(Default)]
struct Output {
    frames: VecDeque<Value>,
    bytes: usize,
    seq: u64,
    closed: bool,
}
pub(super) struct Terminals {
    service: Arc<TerminalService>,
    output: Arc<Mutex<HashMap<String, Output>>>,
    changed: tokio::sync::watch::Sender<u64>,
    stopped: Arc<std::sync::atomic::AtomicBool>,
}
impl Terminals {
    pub fn new() -> Self {
        let output = Arc::new(Mutex::new(HashMap::<String, Output>::new()));
        let (changed, _) = tokio::sync::watch::channel(0);
        let stopped = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stopped_sink = stopped.clone();
        let sink = output.clone();
        let notify = changed.clone();
        let service = Arc::new(TerminalService::new(move |event| {
            let mut all = sink.lock().map_err(|e| e.to_string())?;
            if stopped_sink.load(std::sync::atomic::Ordering::Acquire) {
                return Ok(());
            }
            let (id, data, exit) = match event {
                TerminalEvent::Output { session_id, data } => (session_id, Some(data), None),
                TerminalEvent::Exit { session_id, code } => (session_id, None, Some(code)),
            };
            // Output can arrive before create_session returns its identifier.
            let buffer = all.entry(id).or_default();
            if buffer.closed {
                return Ok(());
            }
            buffer.seq += 1;
            let frame = if let Some(data) = data {
                json!({"type":"data","seq":buffer.seq,"data":STANDARD.encode(data)})
            } else {
                buffer.closed = true;
                json!({"type":"exit","seq":buffer.seq,"exitCode":exit.unwrap_or(0)})
            };
            buffer.bytes += frame.to_string().len();
            buffer.frames.push_back(frame);
            while buffer.bytes > REPLAY_BYTES && buffer.frames.len() > 1 {
                buffer.bytes -= buffer.frames.pop_front().unwrap().to_string().len();
            }
            drop(all);
            notify.send_modify(|n| *n += 1);
            Ok(())
        }));
        Self {
            service,
            output,
            changed,
            stopped,
        }
    }
    pub async fn open(&self, cwd: String, cols: u16, rows: u16) -> Result<RpcReply, RpcError> {
        if self.stopped.load(std::sync::atomic::Ordering::Acquire) {
            return Err(failed("terminal service is closed"));
        }
        let service = self.service.clone();
        let path = cwd.clone();
        let result = tokio::task::spawn_blocking(move || {
            terminal::create_session(service, &path, cols, rows)
        })
        .await
        .map_err(failed)?
        .map_err(failed)?;
        let mut output = self.output.lock().map_err(failed)?;
        if self.stopped.load(std::sync::atomic::Ordering::Acquire) {
            self.service.close(&result.session_id);
            return Err(failed("terminal service is closed"));
        }
        output.entry(result.session_id.clone()).or_default();
        drop(output);
        RpcReply::value(&json!({"id":result.session_id,"cwd":cwd,"shell":"terminal"}))
    }
    pub fn handle(&self, method: &str, params: Value) -> Result<RpcReply, RpcError> {
        let id = params["terminalId"]
            .as_str()
            .ok_or_else(|| failed("terminalId missing"))?
            .to_owned();
        match method {
            methods::WRITE_TERMINAL => {
                let data = STANDARD
                    .decode(
                        params["data"]
                            .as_str()
                            .ok_or_else(|| failed("data missing"))?,
                    )
                    .map_err(failed)?;
                self.service.write(&id, &data).map_err(failed)?;
            }
            methods::RESIZE_TERMINAL => self
                .service
                .resize(
                    &id,
                    dimension(&params, "cols", 80)?,
                    dimension(&params, "rows", 24)?,
                )
                .map_err(failed)?,
            methods::CLOSE_TERMINAL => {
                self.service.close(&id);
                if let Some(buffer) = self.output.lock().map_err(failed)?.get_mut(&id) {
                    buffer.seq += 1;
                    buffer.closed = true;
                    buffer.frames.clear();
                    buffer
                        .frames
                        .push_back(json!({"type":"exit","seq":buffer.seq,"exitCode":0}));
                }
                self.changed.send_modify(|n| *n += 1);
            }
            methods::SUBSCRIBE_TERMINAL => {
                let rx = self.changed.subscribe();
                if !self.output.lock().map_err(failed)?.contains_key(&id) {
                    return Err(failed("terminal not found"));
                }
                let output = self.output.clone();
                let after = params["afterSeq"].as_u64().unwrap_or(0);
                let frames = stream::unfold(
                    (rx, output, id, after),
                    |(mut rx, output, id, mut after)| async move {
                        loop {
                            let (next, closed) = {
                                let all = output.lock().ok()?;
                                let buffer = all.get(&id)?;
                                (
                                    buffer
                                        .frames
                                        .iter()
                                        .find(|v| v["seq"].as_u64().unwrap_or(0) > after)
                                        .cloned(),
                                    buffer.closed,
                                )
                            };
                            if let Some(next) = next {
                                after = next["seq"].as_u64().unwrap();
                                return Some((next, (rx, output, id, after)));
                            }
                            if closed {
                                return None;
                            }
                            rx.changed().await.ok()?;
                        }
                    },
                );
                return Ok(RpcReply::Stream(frames.boxed()));
            }
            _ => return Err(RpcError::UnknownMethod(method.into())),
        }
        RpcReply::value(&json!({}))
    }
    pub fn shutdown(&self) {
        self.stopped
            .store(true, std::sync::atomic::Ordering::Release);
        for id in self.output.lock().unwrap().keys() {
            self.service.close(id);
        }
        self.output.lock().unwrap().clear();
        self.changed.send_modify(|n| *n += 1);
    }
}
impl Drop for Terminals {
    fn drop(&mut self) {
        self.shutdown();
    }
}
pub(super) fn dimension(params: &Value, key: &str, default: u16) -> Result<u16, RpcError> {
    match params.get(key) {
        None => Ok(default),
        Some(value) => value
            .as_u64()
            .and_then(|n| u16::try_from(n).ok())
            .filter(|n| *n > 0)
            .ok_or_else(|| failed(format!("invalid {key}"))),
    }
}
