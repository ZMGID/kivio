use std::{
    future::Future,
    io::{self, Write},
    path::{Path, PathBuf},
    pin::Pin,
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc, Arc, Mutex, OnceLock,
    },
    task::{Context, Poll},
};

use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use uuid::Uuid;

use super::types::StreamFormat;

const FRAME_LIMIT: usize = 16 * 1024;
const RECORD_LIMIT: usize = 1024;
// shortcut: IDs over 256 bytes are omitted, add bounded hashing if attribution is needed.
const CONVERSATION_ID_LIMIT: usize = 256;
const QUEUE_LIMIT: usize = 128;
const FILE_LIMIT: u64 = 2 * 1024 * 1024;
const RETAINED_TAIL: &[u8] = b"{\"event\":\"retained_tail\",\"incomplete\":true}\n";

tokio::task_local! {
    static SPAWN_CAPTURE: ConnectionCapture;
}

pub(crate) struct NativeDiagnostics(Arc<Sink>);
struct Sink {
    directory: PathBuf,
    gate: Arc<AtomicU64>,
    sender: OnceLock<mpsc::SyncSender<(u64, Vec<u8>)>>,
    lost: Arc<AtomicU64>,
}

impl NativeDiagnostics {
    #[cfg(test)]
    pub(crate) fn test_capture(
        protocol: StreamFormat,
    ) -> (Self, ConnectionCapture, mpsc::Receiver<(u64, Vec<u8>)>) {
        let (sender, receiver) = mpsc::sync_channel(QUEUE_LIMIT);
        let diagnostics = Self(Arc::new(Sink {
            directory: PathBuf::new(),
            gate: Arc::new(AtomicU64::new(1)),
            sender: OnceLock::from(sender),
            lost: Arc::new(AtomicU64::new(0)),
        }));
        let capture =
            diagnostics.connection(protocol, Some("conv_550e8400-e29b-41d4-a716-446655440000"));
        (diagnostics, capture, receiver)
    }
    pub(crate) fn new(usage_dir: &Path, enabled: bool) -> Self {
        let diagnostics = Self(Arc::new(Sink {
            directory: usage_dir.join("native-protocol"),
            gate: Arc::new(AtomicU64::new(0)),
            sender: OnceLock::new(),
            lost: Arc::new(AtomicU64::new(0)),
        }));
        diagnostics.set_enabled(enabled);
        diagnostics
    }

    pub(crate) fn set_enabled(&self, enabled: bool) {
        if enabled {
            self.0.sender.get_or_init(|| {
                let (sender, receiver) = mpsc::sync_channel::<(u64, Vec<u8>)>(QUEUE_LIMIT);
                let directory = self.0.directory.clone();
                let gate = self.0.gate.clone();
                let losses = self.0.lost.clone();
                let _ = std::thread::Builder::new()
                    .name("native-diagnostics".into())
                    .spawn(move || {
                        let mut writer = TraceWriter {
                            directory,
                            file: None,
                            bytes: 0,
                        };
                        for (epoch, bytes) in receiver {
                            let lost = serde_json::from_slice::<Value>(&bytes)
                                .ok()
                                .and_then(|value| value["lost_before"].as_u64())
                                .unwrap_or(0)
                                .saturating_add(1);
                            if gate.load(Ordering::Acquire) != epoch {
                                losses.fetch_add(lost, Ordering::Relaxed);
                                continue;
                            }
                            if writer.write(&bytes).is_err() {
                                writer.file = None;
                                losses.fetch_add(lost, Ordering::Relaxed);
                            }
                        }
                    });
                sender
            });
        }
        let _ = self
            .0
            .gate
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |epoch| {
                ((epoch % 2 == 1) != enabled).then_some(epoch.wrapping_add(1))
            });
    }

    pub(crate) fn connection(
        &self,
        protocol: StreamFormat,
        conversation: Option<&str>,
    ) -> ConnectionCapture {
        ConnectionCapture(Arc::new(Connection {
            sink: self.0.clone(),
            protocol,
            conversation: conversation
                .filter(|id| {
                    id.len() <= CONVERSATION_ID_LIMIT
                        && crate::chat::storage::validate_conversation_id(id).is_ok()
                })
                .map(str::to_owned),
            id: Uuid::new_v4(),
            salt: Uuid::new_v4(),
            order: Mutex::new(0),
            observation: Mutex::new(Observation::default()),
        }))
    }
}

struct TraceWriter {
    directory: PathBuf,
    file: Option<std::fs::File>,
    bytes: u64,
}
impl TraceWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
        let current = self.directory.join("current.jsonl");
        if self.file.is_none() {
            std::fs::create_dir_all(&self.directory)?;
            let previous = self.directory.join("previous.jsonl");
            if std::fs::metadata(&previous).is_ok_and(|metadata| metadata.len() > FILE_LIMIT) {
                std::fs::write(previous, RETAINED_TAIL)?;
            }
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&current)?;
            self.bytes = file.metadata()?.len();
            if self.bytes > FILE_LIMIT {
                file = std::fs::File::create(&current)?;
                file.write_all(RETAINED_TAIL)?;
                self.bytes = RETAINED_TAIL.len() as u64;
            }
            self.file = Some(file);
        }
        if self.bytes + bytes.len() as u64 > FILE_LIMIT {
            self.file.take();
            let previous = self.directory.join("previous.jsonl");
            match std::fs::remove_file(&previous) {
                Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
                _ => {}
            }
            std::fs::rename(&current, previous)?;
            self.file = Some(std::fs::File::create(&current)?);
            self.file.as_mut().unwrap().write_all(RETAINED_TAIL)?;
            self.bytes = RETAINED_TAIL.len() as u64;
        }
        self.file.as_mut().unwrap().write_all(bytes)?;
        self.bytes += bytes.len() as u64;
        Ok(())
    }
}

#[derive(Clone)]
pub(crate) struct ConnectionCapture(Arc<Connection>);
struct Connection {
    sink: Arc<Sink>,
    id: Uuid,
    salt: Uuid,
    protocol: StreamFormat,
    conversation: Option<String>,
    order: Mutex<u64>,
    observation: Mutex<Observation>,
}
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct Observation {
    epoch: u64,
    generation: Option<u64>,
}
pub(crate) struct ObservationGuard {
    capture: ConnectionCapture,
    epoch: u64,
}
impl Drop for ObservationGuard {
    fn drop(&mut self) {
        let mut current = self
            .capture
            .0
            .observation
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if current.epoch == self.epoch {
            current.epoch = current.epoch.wrapping_add(1);
            current.generation = None;
        }
    }
}
impl ConnectionCapture {
    pub(crate) fn current() -> Option<Self> {
        SPAWN_CAPTURE.try_with(Clone::clone).ok()
    }
    pub(crate) async fn scope_spawn<F: Future>(&self, future: F) -> F::Output {
        SPAWN_CAPTURE.scope(self.clone(), future).await
    }
    pub(crate) fn observe_generation(&self, generation: u64) -> ObservationGuard {
        let mut current = self.0.observation.lock().unwrap_or_else(|e| e.into_inner());
        current.epoch = current.epoch.wrapping_add(1);
        current.generation = Some(generation);
        ObservationGuard {
            capture: self.clone(),
            epoch: current.epoch,
        }
    }
    fn observation(&self) -> Observation {
        *self.0.observation.lock().unwrap_or_else(|e| e.into_inner())
    }
    fn record(
        &self,
        direction: Direction,
        observation: Option<Observation>,
        boundary: bool,
        event: Event,
    ) {
        let epoch = self.0.sink.gate.load(Ordering::Acquire);
        if epoch % 2 == 0 {
            return;
        }
        let Some(sender) = self.0.sink.sender.get() else {
            return;
        };
        let mut sequence = self.0.order.lock().unwrap_or_else(|e| e.into_inner());
        *sequence = sequence.wrapping_add(1);
        let lost = self.0.sink.lost.swap(0, Ordering::Relaxed);
        let record = Record {
            schema: 1,
            connection: self.0.id.to_string(),
            protocol: self.0.protocol,
            observed_at_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
            conversation: self.0.conversation.as_deref(),
            sequence: *sequence,
            observed_during_generation: observation.and_then(|o| o.generation),
            observation_boundary: boundary,
            direction,
            lost_before: lost,
            event,
        };
        let bytes = serde_json::to_vec(&record)
            .ok()
            .filter(|v| v.len() < RECORD_LIMIT);
        if let Some(mut bytes) = bytes {
            bytes.push(b'\n');
            if sender.try_send((epoch, bytes)).is_ok() {
                return;
            }
        }
        self.0
            .sink
            .lost
            .fetch_add(lost.saturating_add(1), Ordering::Relaxed);
    }
    fn hash_id(&self, value: Option<&Value>) -> Option<(&'static str, String)> {
        let value = value?;
        let (kind, text) = match value {
            Value::String(text) => ("string", text.clone()),
            Value::Number(number) => ("number", number.to_string()),
            _ => return None,
        };
        let mut hash = Sha256::new();
        hash.update(self.0.salt.as_bytes());
        hash.update(kind.as_bytes());
        hash.update(text.as_bytes());
        let hash = hash.finalize()[..16]
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        Some((kind, hash))
    }
    fn summarize(&self, bytes: &[u8], count: u64) -> Event {
        let Ok(value) = serde_json::from_slice::<Value>(bytes) else {
            return Event::Omitted {
                bytes: count,
                reason: "malformed",
            };
        };
        if !value.is_object() {
            return Event::Omitted {
                bytes: count,
                reason: "non_object",
            };
        }
        Event::Frame {
            bytes: count,
            method: known(value.get("method"), METHODS),
            kind: known(value.get("type"), KINDS),
            status: known(
                value
                    .pointer("/params/turn/status")
                    .or_else(|| value.get("status")),
                STATUSES,
            ),
            rpc_id: self.hash_id(first(
                &value,
                &["/id", "/request_id", "/response/request_id"],
            )),
            session_id: self.hash_id(first(
                &value,
                &[
                    "/session_id",
                    "/params/sessionId",
                    "/result/sessionId",
                    "/params/session_id",
                ],
            )),
            thread_id: self.hash_id(first(
                &value,
                &["/params/threadId", "/params/thread/id", "/result/thread/id"],
            )),
            turn_id: self.hash_id(first(
                &value,
                &["/params/turnId", "/params/turn/id", "/result/turn/id"],
            )),
        }
    }
}

fn first<'a>(value: &'a Value, paths: &[&str]) -> Option<&'a Value> {
    paths.iter().find_map(|path| value.pointer(path))
}
fn known(value: Option<&Value>, allowed: &'static [&'static str]) -> Option<&'static str> {
    let value = value?.as_str()?;
    allowed
        .iter()
        .copied()
        .find(|candidate| *candidate == value)
}
const METHODS: &[&str] = &[
    "initialize",
    "initialized",
    "authenticate",
    "session/new",
    "session/load",
    "session/list",
    "session/prompt",
    "session/cancel",
    "session/update",
    "session/request_permission",
    "session/open",
    "session/ask",
    "session/stop",
    "session/configure",
    "session/settings",
    "session/fork",
    "thread/start",
    "thread/resume",
    "thread/fork",
    "thread/read",
    "thread/revert",
    "thread/turns/list",
    "turn/start",
    "turn/steer",
    "turn/interrupt",
    "turn/started",
    "turn/completed",
    "item/started",
    "item/completed",
    "item/agentMessage/delta",
    "item/reasoning/textDelta",
    "item/commandExecution/requestApproval",
    "item/fileChange/requestApproval",
    "item/tool/requestUserInput",
    "assistant/chunk",
    "tool/call",
    "tool/result",
    "turn/end",
    "error",
];
const KINDS: &[&str] = &[
    "system",
    "user",
    "assistant",
    "result",
    "stream_event",
    "control_request",
    "control_response",
    "control_cancel_request",
    "keep_alive",
    "prompt",
    "response",
    "agent_start",
    "agent_end",
    "turn_start",
    "turn_end",
    "message_start",
    "message_update",
    "message_end",
    "tool_execution_start",
    "tool_execution_update",
    "tool_execution_end",
    "extension_ui_request",
    "extension_ui_response",
    "abort",
    "steer",
    "follow_up",
    "get_state",
    "get_commands",
    "set_model",
    "set_thinking_level",
    "new_session",
    "switch_session",
    "fork",
    "compact",
    "error",
];
const STATUSES: &[&str] = &[
    "completed",
    "interrupted",
    "failed",
    "inProgress",
    "success",
    "error",
];

#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Direction {
    Stdin,
    Stdout,
    Stderr,
}
#[derive(Serialize)]
struct Record<'a> {
    schema: u8,
    observed_at_ms: u64,
    connection: String,
    protocol: StreamFormat,
    #[serde(skip_serializing_if = "Option::is_none")]
    conversation: Option<&'a str>,
    sequence: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    observed_during_generation: Option<u64>,
    observation_boundary: bool,
    direction: Direction,
    lost_before: u64,
    #[serde(flatten)]
    event: Event,
}
#[derive(Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
enum Event {
    Frame {
        bytes: u64,
        #[serde(skip_serializing_if = "Option::is_none")]
        method: Option<&'static str>,
        #[serde(skip_serializing_if = "Option::is_none")]
        kind: Option<&'static str>,
        #[serde(skip_serializing_if = "Option::is_none")]
        status: Option<&'static str>,
        #[serde(skip_serializing_if = "Option::is_none")]
        rpc_id: Option<(&'static str, String)>,
        #[serde(skip_serializing_if = "Option::is_none")]
        session_id: Option<(&'static str, String)>,
        #[serde(skip_serializing_if = "Option::is_none")]
        thread_id: Option<(&'static str, String)>,
        #[serde(skip_serializing_if = "Option::is_none")]
        turn_id: Option<(&'static str, String)>,
    },
    Omitted {
        bytes: u64,
        reason: &'static str,
    },
    CaptureStarted {
        late: bool,
    },
    Eof,
    IoFailure {
        kind: &'static str,
    },
}

pub type AgentStdin = AgentPipe<tokio::process::ChildStdin>;
pub type AgentStdout = AgentPipe<tokio::process::ChildStdout>;
pub type AgentStderr = AgentPipe<tokio::process::ChildStderr>;

pub struct AgentPipe<T> {
    inner: T,
    capture: Option<ConnectionCapture>,
    direction: Direction,
    gate: u64,
    started: bool,
    ended: bool,
    frame: Frame,
}
#[derive(Default)]
struct Frame {
    prefix: Vec<u8>,
    bytes: u64,
    first: Observation,
    boundary: bool,
    incomplete: bool,
}
impl<T> AgentPipe<T> {
    pub(crate) fn new(inner: T, direction: Direction, capture: Option<ConnectionCapture>) -> Self {
        let gate = capture
            .as_ref()
            .map_or(0, |c| c.0.sink.gate.load(Ordering::Acquire));
        Self {
            inner,
            capture,
            direction,
            gate,
            started: false,
            ended: false,
            frame: Frame::default(),
        }
    }
    fn accept(&mut self, bytes: &[u8]) {
        let Some(capture) = self.capture.clone() else {
            return;
        };
        let gate = capture.0.sink.gate.load(Ordering::Acquire);
        if gate != self.gate {
            let previous = std::mem::take(&mut self.frame);
            self.frame.incomplete = previous.bytes != 0 || previous.incomplete;
            if previous.bytes != 0 {
                capture.record(
                    self.direction,
                    None,
                    true,
                    Event::Omitted {
                        bytes: previous.bytes,
                        reason: "gate_changed",
                    },
                );
            }
            self.gate = gate;
            self.started = false;
        }
        if gate % 2 == 0 {
            if let Some(last) = bytes.iter().rposition(|b| *b == b'\n') {
                self.frame.incomplete = last + 1 != bytes.len();
            } else if !bytes.is_empty() {
                self.frame.incomplete = true;
            }
            return;
        }
        if !self.started {
            capture.record(
                self.direction,
                None,
                false,
                Event::CaptureStarted {
                    late: gate > 1 || self.frame.incomplete,
                },
            );
            self.started = true;
        }
        let observation = capture.observation();
        for byte in bytes {
            if self.frame.bytes == 0 {
                self.frame.first = observation;
            }
            self.frame.boundary |= self.frame.first != observation;
            self.frame.bytes = self.frame.bytes.saturating_add(1);
            if *byte == b'\n' {
                self.finish(None);
            } else if self.direction != Direction::Stderr && !self.frame.incomplete {
                if self.frame.bytes <= FRAME_LIMIT as u64 {
                    self.frame.prefix.push(*byte);
                } else {
                    self.frame.prefix.clear();
                }
            }
        }
    }
    fn finish(&mut self, reason: Option<&'static str>) {
        let frame = std::mem::take(&mut self.frame);
        if frame.bytes == 0 {
            return;
        }
        let Some(capture) = &self.capture else { return };
        let reason = reason.or_else(|| {
            if frame.incomplete {
                Some("partial_capture")
            } else if self.direction == Direction::Stderr {
                Some("stderr")
            } else if frame.bytes > FRAME_LIMIT as u64 {
                Some("oversized")
            } else {
                None
            }
        });
        let boundary = frame.boundary || frame.first != capture.observation();
        let event = match reason {
            Some(reason) => Event::Omitted {
                bytes: frame.bytes,
                reason,
            },
            None => capture.summarize(&frame.prefix, frame.bytes),
        };
        capture.record(
            self.direction,
            (!boundary).then_some(frame.first),
            boundary,
            event,
        );
    }
    fn end(&mut self, error: Option<io::ErrorKind>) {
        if self.ended {
            return;
        }
        self.ended = true;
        self.accept(&[]);
        self.finish(Some("incomplete"));
        if let Some(capture) = &self.capture {
            let event = match error {
                None => Event::Eof,
                Some(kind) => Event::IoFailure {
                    kind: match kind {
                        io::ErrorKind::BrokenPipe => "broken_pipe",
                        io::ErrorKind::UnexpectedEof => "unexpected_eof",
                        io::ErrorKind::ConnectionReset => "connection_reset",
                        io::ErrorKind::Interrupted => "interrupted",
                        _ => "other",
                    },
                },
            };
            capture.record(self.direction, Some(capture.observation()), false, event);
        }
    }
}
impl<T> Drop for AgentPipe<T> {
    fn drop(&mut self) {
        self.accept(&[]);
        self.finish(Some("dropped_partial"));
    }
}
impl<T: AsyncRead + Unpin> AsyncRead for AgentPipe<T> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let before = buf.filled().len();
        let remaining = buf.remaining();
        let result = Pin::new(&mut this.inner).poll_read(cx, buf);
        if let Poll::Ready(outcome) = &result {
            this.accept(&buf.filled()[before..]);
            match outcome {
                Err(error) => this.end(Some(error.kind())),
                Ok(()) if remaining != 0 && before == buf.filled().len() => this.end(None),
                _ => {}
            }
        }
        result
    }
}
impl<T: AsyncWrite + Unpin> AsyncWrite for AgentPipe<T> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        let result = Pin::new(&mut this.inner).poll_write(cx, buf);
        match &result {
            Poll::Ready(Ok(count)) => this.accept(&buf[..*count]),
            Poll::Ready(Err(error)) => this.end(Some(error.kind())),
            _ => {}
        }
        result
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let result = Pin::new(&mut this.inner).poll_flush(cx);
        if let Poll::Ready(Err(error)) = &result {
            this.end(Some(error.kind()));
        }
        result
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let result = Pin::new(&mut this.inner).poll_shutdown(cx);
        if let Poll::Ready(Err(error)) = &result {
            this.end(Some(error.kind()));
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn records(receiver: &mpsc::Receiver<(u64, Vec<u8>)>) -> Vec<Value> {
        receiver
            .try_iter()
            .map(|(_, bytes)| {
                assert!(bytes.len() <= RECORD_LIMIT);
                assert!(!String::from_utf8_lossy(&bytes).contains("SECRET"));
                serde_json::from_slice(&bytes).unwrap()
            })
            .collect()
    }

    #[test]
    fn conversation_ids_are_validated_and_bounded_before_recording() {
        let normal = "conv_550e8400-e29b-41d4-a716-446655440000";
        let automation = "conv_auto_auto-1_n-2";
        let at_limit = format!("conv_{}", "x".repeat(251));
        let over_limit = format!("conv_{}", "x".repeat(252));
        for (id, expected) in [
            (Some(normal), Some(normal)),
            (Some(automation), Some(automation)),
            (Some(at_limit.as_str()), Some(at_limit.as_str())),
            (Some(over_limit.as_str()), None),
            (Some("conv_"), None),
            (Some("unrelated"), None),
            (Some("conv_../SECRET"), None),
            (Some("conv_x\nSECRET"), None),
            (None, None),
        ] {
            let (diagnostics, _, receiver) =
                NativeDiagnostics::test_capture(StreamFormat::AcpJsonRpc);
            let capture = diagnostics.connection(StreamFormat::AcpJsonRpc, id);
            let mut pipe = AgentPipe::new((), Direction::Stdout, Some(capture));
            pipe.accept(b"{}\n");

            let records = records(&receiver);
            assert!(records.iter().any(|record| record["event"] == "frame"));
            for record in records {
                assert_eq!(record.get("conversation").and_then(Value::as_str), expected);
                if expected.is_none() {
                    assert!(record.get("conversation").is_none());
                }
            }
        }
    }

    struct ShortIo {
        bytes: Vec<u8>,
        offset: usize,
        pending: bool,
        fail_after: Option<usize>,
    }
    impl AsyncRead for ShortIo {
        fn poll_read(
            mut self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            buf: &mut ReadBuf<'_>,
        ) -> Poll<io::Result<()>> {
            self.pending = !self.pending;
            if self.pending {
                cx.waker().wake_by_ref();
                return Poll::Pending;
            }
            let count = 3.min(buf.remaining()).min(self.bytes.len() - self.offset);
            buf.put_slice(&self.bytes[self.offset..self.offset + count]);
            self.offset += count;
            Poll::Ready(Ok(()))
        }
    }
    impl AsyncWrite for ShortIo {
        fn poll_write(
            mut self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            bytes: &[u8],
        ) -> Poll<io::Result<usize>> {
            self.pending = !self.pending;
            if self.pending {
                cx.waker().wake_by_ref();
                return Poll::Pending;
            }
            if self.fail_after.is_some_and(|max| self.bytes.len() >= max) {
                return Poll::Ready(Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "SECRET error",
                )));
            }
            let count = 2.min(bytes.len());
            self.bytes.extend_from_slice(&bytes[..count]);
            Poll::Ready(Ok(count))
        }
        fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
        fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    #[tokio::test]
    async fn fragmented_reads_and_short_pending_writes_preserve_bytes_and_redact() {
        let (_, capture, receiver) = NativeDiagnostics::test_capture(StreamFormat::CodexAppServer);
        let bytes = b"{\"method\":\"initialize\",\"id\":\"SECRET-id\",\"SECRET-key\":\"SECRET-body\"}\n{\"id\":\"SECRET-id\",\"result\":{}}\n".to_vec();
        let mut input = AgentPipe::new(
            ShortIo {
                bytes: bytes.clone(),
                offset: 0,
                pending: false,
                fail_after: None,
            },
            Direction::Stdout,
            Some(capture.clone()),
        );
        let mut read = Vec::new();
        input.read_to_end(&mut read).await.unwrap();
        assert_eq!(read, bytes);
        let mut output = AgentPipe::new(
            ShortIo {
                bytes: vec![],
                offset: 0,
                pending: false,
                fail_after: None,
            },
            Direction::Stdin,
            Some(capture.clone()),
        );
        output.write_all(&bytes).await.unwrap();
        output.flush().await.unwrap();
        output.shutdown().await.unwrap();
        assert_eq!(output.inner.bytes, bytes);
        let records = records(&receiver);
        let frames: Vec<_> = records.iter().filter(|v| v["event"] == "frame").collect();
        assert_eq!(frames.len(), 4);
        assert_eq!(frames[0]["rpc_id"], frames[1]["rpc_id"]);
        assert_eq!(frames[0]["rpc_id"], frames[2]["rpc_id"]);
        assert_eq!(frames[0]["rpc_id"][0], "string");
        assert_eq!(frames[0]["method"], "initialize");
        assert_ne!(
            capture.hash_id(Some(&Value::from(1))),
            capture.hash_id(Some(&Value::from("1")))
        );
        let (_, other, _) = NativeDiagnostics::test_capture(StreamFormat::CodexAppServer);
        assert_ne!(
            capture.hash_id(Some(&Value::from("1"))),
            other.hash_id(Some(&Value::from("1")))
        );
    }

    #[tokio::test]
    async fn partial_write_failure_records_only_accepted_bytes_and_preserves_error() {
        let (_, capture, receiver) = NativeDiagnostics::test_capture(StreamFormat::PiRpc);
        let mut output = AgentPipe::new(
            ShortIo {
                bytes: vec![],
                offset: 0,
                pending: false,
                fail_after: Some(4),
            },
            Direction::Stdin,
            Some(capture),
        );
        let error = output.write_all(b"SECRET-unaccepted\n").await.unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
        assert_eq!(error.to_string(), "SECRET error");
        assert_eq!(output.inner.bytes, b"SECR");
        let records = records(&receiver);
        assert!(records
            .iter()
            .any(|v| v["reason"] == "incomplete" && v["bytes"] == 4));
        assert!(records
            .iter()
            .any(|v| v["event"] == "io_failure" && v["kind"] == "broken_pipe"));
    }

    #[test]
    fn malformed_oversized_stderr_and_partial_frames_never_store_raw_content() {
        let (_, capture, receiver) = NativeDiagnostics::test_capture(StreamFormat::AcpJsonRpc);
        let mut pipe = AgentPipe::new((), Direction::Stdout, Some(capture.clone()));
        pipe.accept(b"SECRET-not-json\n\xff\n{\"method\":\"SECRET-method\",\"type\":\"SECRET-type\",\"status\":\"SECRET-status\"}\n");
        pipe.accept(&vec![b'x'; FRAME_LIMIT + 200]);
        assert!(pipe.frame.prefix.is_empty());
        assert!(pipe.frame.prefix.capacity() <= FRAME_LIMIT);
        pipe.accept(b"SECRET\n");
        pipe.accept(b"SECRET-tail");
        pipe.end(None);
        let mut stderr = AgentPipe::new((), Direction::Stderr, Some(capture));
        stderr.accept(b"{\"method\":\"initialize\",\"SECRET\":\"stderr\"}\n");
        assert!(stderr.frame.prefix.is_empty());
        let records = records(&receiver);
        for reason in ["malformed", "oversized", "incomplete", "stderr"] {
            assert!(records.iter().any(|v| v["reason"] == reason));
        }
        let unknown = records.iter().find(|v| v["event"] == "frame").unwrap();
        assert!(
            unknown.get("method").is_none()
                && unknown.get("kind").is_none()
                && unknown.get("status").is_none()
        );
    }

    #[test]
    fn gating_and_observation_boundaries_do_not_attribute_old_or_idle_bytes() {
        let (diagnostics, capture, receiver) = NativeDiagnostics::test_capture(StreamFormat::PiRpc);
        diagnostics.set_enabled(false);
        let mut pipe = AgentPipe::new((), Direction::Stdout, Some(capture.clone()));
        pipe.accept(b"SECRET-disabled");
        assert!(pipe.frame.prefix.capacity() == 0 && receiver.try_recv().is_err());
        diagnostics.set_enabled(true);
        pipe.accept(b"\n");
        let old = capture.observe_generation(1);
        pipe.accept(b"{\"type\":");
        let new = capture.observe_generation(2);
        drop(old);
        assert_eq!(capture.observation().generation, Some(2));
        pipe.accept(b"\"response\"}\n");
        pipe.accept(b"{\"type\":\"response\"}\n");
        drop(new);
        pipe.accept(b"{}\n");
        pipe.accept(b"{\"id\":\"SECRET-old\"");
        diagnostics.set_enabled(false);
        diagnostics.set_enabled(true);
        drop(pipe);
        let records = records(&receiver);
        assert!(records
            .iter()
            .any(|v| v["event"] == "capture_started" && v["late"] == true));
        assert!(records.iter().any(|v| v["reason"] == "partial_capture"));
        let frames: Vec<_> = records.iter().filter(|v| v["event"] == "frame").collect();
        assert_eq!(frames.len(), 3);
        assert_eq!(frames[0]["observation_boundary"], true);
        assert!(frames[0].get("observed_during_generation").is_none());
        assert_eq!(frames[1]["observed_during_generation"], 2);
        assert!(frames[2].get("observed_during_generation").is_none());
        assert!(records.iter().any(|v| v["reason"] == "gate_changed"));
    }

    #[tokio::test]
    async fn queue_loss_and_file_rotation_are_bounded_and_failures_are_local() {
        let (_, capture, receiver) = NativeDiagnostics::test_capture(StreamFormat::PiRpc);
        let mut pipe = AgentPipe::new((), Direction::Stdout, Some(capture));
        for _ in 0..QUEUE_LIMIT + 10 {
            pipe.accept(b"{}\n");
        }
        assert_eq!(records(&receiver).len(), QUEUE_LIMIT);
        pipe.accept(b"{}\n");
        assert_eq!(records(&receiver)[0]["lost_before"], 11);
        let dir = tempfile::tempdir().unwrap();
        let mut writer = TraceWriter {
            directory: dir.path().join("trace"),
            file: None,
            bytes: 0,
        };
        std::fs::create_dir_all(&writer.directory).unwrap();
        for name in ["current.jsonl", "previous.jsonl"] {
            std::fs::File::create(writer.directory.join(name))
                .unwrap()
                .set_len(FILE_LIMIT + 1)
                .unwrap();
        }
        let bytes = vec![b' '; RECORD_LIMIT];
        for _ in 0..(FILE_LIMIT as usize / RECORD_LIMIT * 3 + 1) {
            writer.write(&bytes).unwrap();
        }
        assert_eq!(std::fs::read_dir(&writer.directory).unwrap().count(), 2);
        for entry in std::fs::read_dir(&writer.directory).unwrap() {
            assert!(entry.unwrap().metadata().unwrap().len() <= FILE_LIMIT);
        }
        assert!(
            std::fs::read_to_string(writer.directory.join("current.jsonl"))
                .unwrap()
                .starts_with("{\"event\":\"retained_tail\"")
        );
        let blocked = dir.path().join("blocked");
        std::fs::write(&blocked, "not a directory").unwrap();
        let mut writer = TraceWriter {
            directory: blocked.clone(),
            file: None,
            bytes: 0,
        };
        assert!(writer.write(b"{}\n").is_err());
        let diagnostics = NativeDiagnostics::new(&blocked, true);
        let capture = diagnostics.connection(StreamFormat::PiRpc, None);
        let mut pipe = AgentPipe::new(Vec::new(), Direction::Stdin, Some(capture));
        pipe.write_all(b"{\"type\":\"prompt\",\"message\":\"SECRET\"}\n")
            .await
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            while diagnostics.0.lost.load(Ordering::Relaxed) == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            pipe.inner,
            b"{\"type\":\"prompt\",\"message\":\"SECRET\"}\n"
        );
    }
}
