use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex, Weak,
    },
    time::Duration,
};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::{
    io::{AsyncBufRead, AsyncReadExt, AsyncWriteExt, BufReader},
    process::{ChildStdin, Command},
    sync::{mpsc, Notify},
    task::JoinHandle,
    time::{timeout, Instant},
};

const MAX_FILE_BYTES: u64 = 1024 * 1024;
const MAX_FRAME_BYTES: usize = 2 * 1024 * 1024;
const MAX_HEADER_BYTES: usize = 8192;
const MAX_DOCUMENTS: usize = 128;
const MAX_SESSIONS: usize = 16;
const IO_TIMEOUT: Duration = Duration::from_secs(2);
const INITIALIZE_TIMEOUT: Duration = Duration::from_secs(10);
const IDLE_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub struct LspServerConfig {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    pub language_id: String,
    pub extensions: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LspPosition {
    pub line: u32,
    pub character: u32,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LspRange {
    pub start: LspPosition,
    pub end: LspPosition,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LspDiagnostic {
    pub range: LspRange,
    pub severity: Option<u8>,
    pub source: Option<String>,
    pub message: String,
}

#[derive(Clone)]
pub struct LspDiagnostics {
    pub owner: String,
    pub workspace: PathBuf,
    pub path: PathBuf,
    pub uri: String,
    pub generation: u64,
    pub version: i32,
    pub diagnostics: Vec<LspDiagnostic>,
    state: Weak<SessionState>,
}

impl LspDiagnostics {
    pub fn is_current(&self) -> bool {
        let Some(state) = self.state.upgrade() else {
            return false;
        };
        state.active.load(Ordering::Acquire)
            && state.generation == self.generation
            && state
                .documents
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .get(&self.uri)
                .is_some_and(|document| document.present && document.version == self.version)
    }
}

pub type DiagnosticsCallback = Arc<dyn Fn(LspDiagnostics) + Send + Sync>;

#[derive(Default, Clone)]
pub struct LspService {
    registry: Arc<Registry>,
}

#[derive(Default)]
struct Registry {
    sessions: Mutex<HashMap<SessionKey, Session>>,
    generation: AtomicU64,
}

#[derive(Clone, Hash, PartialEq, Eq)]
struct SessionKey {
    owner: String,
    workspace: PathBuf,
    config: LspServerConfig,
}

struct Session {
    state: Arc<SessionState>,
    changes: mpsc::Sender<Change>,
    task: JoinHandle<()>,
}

struct SessionState {
    owner: String,
    workspace: PathBuf,
    generation: u64,
    active: AtomicBool,
    stop: Notify,
    documents: Mutex<HashMap<String, Document>>,
}

struct Document {
    path: PathBuf,
    version: i32,
    present: bool,
    emit: DiagnosticsCallback,
}

enum Change {
    Content {
        uri: String,
        version: i32,
        text: String,
    },
    Closed {
        uri: String,
    },
}

impl SessionState {
    fn close(&self) {
        self.active.store(false, Ordering::Release);
        self.stop.notify_one();
    }
}

impl Drop for Registry {
    fn drop(&mut self) {
        for session in self
            .sessions
            .get_mut()
            .unwrap_or_else(|e| e.into_inner())
            .values()
        {
            session.state.close();
        }
    }
}

impl LspService {
    pub async fn file_changed(
        &self,
        owner: &str,
        workspace: &Path,
        path: &Path,
        config: &LspServerConfig,
        emit: DiagnosticsCallback,
    ) -> Result<(), String> {
        if config.command.trim().is_empty() || config.language_id.trim().is_empty() {
            return Err("LSP command and languageId must be configured".into());
        }
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("");
        if !config
            .extensions
            .iter()
            .any(|item| item.trim_start_matches('.').eq_ignore_ascii_case(extension))
        {
            return Ok(());
        }
        let workspace = tokio::fs::canonicalize(workspace)
            .await
            .map_err(|e| e.to_string())?;
        let path = tokio::fs::canonicalize(path)
            .await
            .map_err(|e| e.to_string())?;
        if !path.starts_with(&workspace) {
            return Err("LSP file is outside the authorized workspace".into());
        }
        let uri = url::Url::from_file_path(&path)
            .map_err(|_| "Invalid LSP file path")?
            .to_string();
        let key = SessionKey {
            owner: owner.into(),
            workspace: workspace.clone(),
            config: config.clone(),
        };
        let (state, changes) = {
            let mut sessions = self
                .registry
                .sessions
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if sessions
                .get(&key)
                .is_some_and(|session| !session.state.active.load(Ordering::Acquire))
            {
                if let Some(old) = sessions.remove(&key) {
                    old.state.close();
                }
            }
            if !sessions.contains_key(&key) {
                sessions.retain(|_, session| session.state.active.load(Ordering::Acquire));
                if sessions.len() >= MAX_SESSIONS {
                    return Err("Too many active LSP servers".into());
                }
                let state = Arc::new(SessionState {
                    owner: owner.into(),
                    workspace,
                    generation: self.registry.generation.fetch_add(1, Ordering::Relaxed) + 1,
                    active: AtomicBool::new(true),
                    stop: Notify::new(),
                    documents: Mutex::new(HashMap::new()),
                });
                let (changes, receiver) = mpsc::channel(32);
                let worker_state = state.clone();
                let worker_config = config.clone();
                let task = tokio::spawn(async move {
                    let _lifetime = SessionLifetime(worker_state.clone());
                    if let Err(error) = run_server(worker_state, worker_config, receiver).await {
                        eprintln!("LSP server stopped: {error}");
                    }
                });
                sessions.insert(
                    key.clone(),
                    Session {
                        state,
                        changes,
                        task,
                    },
                );
            }
            let session = sessions.get(&key).expect("registered LSP server");
            (session.state.clone(), session.changes.clone())
        };
        let version = {
            let mut documents = state.documents.lock().unwrap_or_else(|e| e.into_inner());
            if documents.len() >= MAX_DOCUMENTS && !documents.contains_key(&uri) {
                return Err("Too many open LSP documents".into());
            }
            let version = documents
                .get(&uri)
                .map_or(Some(1), |document| document.version.checked_add(1))
                .ok_or("LSP document version exhausted")?;
            documents.insert(
                uri.clone(),
                Document {
                    path: path.clone(),
                    version,
                    present: true,
                    emit,
                },
            );
            version
        };
        let file = tokio::fs::File::open(&path)
            .await
            .map_err(|e| e.to_string())?;
        let mut bytes = Vec::new();
        file.take(MAX_FILE_BYTES + 1)
            .read_to_end(&mut bytes)
            .await
            .map_err(|e| e.to_string())?;
        if bytes.len() as u64 > MAX_FILE_BYTES {
            return Err("LSP document exceeds 1 MiB".into());
        }
        let text = String::from_utf8(bytes).map_err(|_| "LSP document is not UTF-8")?;
        changes
            .try_send(Change::Content { uri, version, text })
            .map_err(|error| format!("LSP change was not queued: {error}"))
    }

    pub async fn file_removed(
        &self,
        owner: &str,
        workspace: &Path,
        path: &Path,
    ) -> Result<(), String> {
        let workspace = tokio::fs::canonicalize(workspace)
            .await
            .map_err(|e| e.to_string())?;
        if !path.starts_with(&workspace) {
            return Err("LSP file is outside the authorized workspace".into());
        }
        let uri = url::Url::from_file_path(path)
            .map_err(|_| "Invalid LSP file path")?
            .to_string();
        for (key, session) in self
            .registry
            .sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
        {
            if key.owner == owner && key.workspace == workspace {
                if let Some(document) = session
                    .state
                    .documents
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .get_mut(&uri)
                {
                    document.present = false;
                    let _ = session
                        .changes
                        .try_send(Change::Closed { uri: uri.clone() });
                }
            }
        }
        Ok(())
    }

    pub(crate) fn retain_configurations(&self, owner: &str, configs: &[LspServerConfig]) {
        self.registry
            .sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|key, session| {
                if key.owner == owner && !configs.contains(&key.config) {
                    session.state.close();
                    false
                } else {
                    true
                }
            });
    }

    pub fn close_owner(&self, owner: &str) {
        self.registry
            .sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|key, session| {
                if key.owner == owner {
                    session.state.close();
                    false
                } else {
                    true
                }
            });
    }

    pub async fn shutdown(&self) {
        let mut sessions: Vec<_> = self
            .registry
            .sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .drain()
            .map(|(_, session)| session)
            .collect();
        for session in &sessions {
            session.state.close();
        }
        if timeout(Duration::from_secs(3), async {
            for session in &mut sessions {
                let _ = (&mut session.task).await;
            }
        })
        .await
        .is_err()
        {
            for session in &sessions {
                session.task.abort();
            }
        }
    }
}

struct SessionLifetime(Arc<SessionState>);

impl Drop for SessionLifetime {
    fn drop(&mut self) {
        self.0.close();
    }
}

struct OwnedProcess {
    child: Box<dyn process_wrap::tokio::ChildWrapper>,
    killed: bool,
}

impl OwnedProcess {
    fn kill(&mut self) {
        if !self.killed {
            let _ = self.child.start_kill();
            self.killed = true;
        }
    }
}

impl Drop for OwnedProcess {
    fn drop(&mut self) {
        self.kill();
    }
}

struct ReaderTask(JoinHandle<()>);

impl Drop for ReaderTask {
    fn drop(&mut self) {
        self.0.abort();
    }
}

async fn run_server(
    state: Arc<SessionState>,
    config: LspServerConfig,
    mut changes: mpsc::Receiver<Change>,
) -> Result<(), String> {
    if !state.active.load(Ordering::Acquire) {
        return Ok(());
    }
    let mut command = Command::new(&config.command);
    command
        .args(&config.args)
        .current_dir(&state.workspace)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut wrapped = process_wrap::tokio::CommandWrap::from(command);
    #[cfg(unix)]
    wrapped.wrap(process_wrap::tokio::ProcessGroup::leader());
    #[cfg(windows)]
    {
        let mut flags = process_wrap::tokio::CreationFlags(Default::default());
        flags.0 .0 = 0x08000000;
        wrapped.wrap(flags).wrap(process_wrap::tokio::JobObject);
    }
    wrapped.wrap(process_wrap::tokio::KillOnDrop);
    let mut process = OwnedProcess {
        child: wrapped.spawn().map_err(|e| e.to_string())?,
        killed: false,
    };
    let mut stdin = process
        .child
        .stdin()
        .take()
        .ok_or("LSP server has no stdin")?;
    let stdout = process
        .child
        .stdout()
        .take()
        .ok_or("LSP server has no stdout")?;
    let (incoming, mut messages) = mpsc::channel(16);
    let _reader = ReaderTask(tokio::spawn(async move {
        let mut reader = BufReader::new(stdout);
        loop {
            let frame = read_frame(&mut reader).await;
            let failed = frame.is_err();
            if incoming.send(frame).await.is_err() || failed {
                break;
            }
        }
    }));
    let root_uri = url::Url::from_directory_path(&state.workspace)
        .map_err(|_| "Invalid LSP workspace path")?
        .to_string();
    let mut initialized = false;
    let result = tokio::select! {
        _ = state.stop.notified() => Ok(()),
        result = async {
            let pull = timeout(INITIALIZE_TIMEOUT, async {
                write_frame(&mut stdin, json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
                    "processId":std::process::id(),"rootUri":root_uri,
                    "workspaceFolders":[{"uri":root_uri,"name":"workspace"}],
                    "capabilities":{"textDocument":{"publishDiagnostics":{"versionSupport":true},"diagnostic":{"dynamicRegistration":false,"relatedDocumentSupport":false}}}
                }})).await?;
                loop {
                    let message = messages.recv().await.ok_or("LSP server closed during initialize")??;
                    if message.get("id").and_then(Value::as_i64) == Some(1) && message.get("method").is_none() {
                        if message.get("error").is_some() {
                            return Err("LSP initialize failed".to_owned());
                        }
                        if !message.pointer("/result/capabilities").is_some_and(Value::is_object) {
                            return Err("Invalid LSP initialize response".into());
                        }
                        return Ok(message.pointer("/result/capabilities/diagnosticProvider").is_some_and(|value| !value.is_null() && value != &Value::Bool(false)));
                    }
                    answer_server_request(&mut stdin, &message, &root_uri).await?;
                }
            }).await.map_err(|_| "LSP initialize timed out")??;
            write_frame(&mut stdin, json!({"jsonrpc":"2.0","method":"initialized","params":{}})).await?;
            initialized = true;
            let mut opened = HashMap::<String, i32>::new();
            let mut pending = HashMap::<i64, (String, i32, Instant)>::new();
            let mut request_id = 2_i64;
            let idle = tokio::time::sleep(IDLE_TIMEOUT);
            tokio::pin!(idle);
            let mut expiry = tokio::time::interval(Duration::from_secs(1));
            loop {
                tokio::select! {
                    change = changes.recv() => {
                        let Some(change) = change else { return Ok(()); };
                        idle.as_mut().reset(Instant::now() + IDLE_TIMEOUT);
                        let (uri, version, text) = match change {
                            Change::Content { uri, version, text } => (uri, version, text),
                            Change::Closed { uri } => {
                                if opened.remove(&uri).is_some() {
                                    write_frame(&mut stdin, json!({"jsonrpc":"2.0","method":"textDocument/didClose","params":{"textDocument":{"uri":uri}}})).await?;
                                }
                                pending.retain(|_, (pending_uri, _, _)| pending_uri != &uri);
                                continue;
                            }
                        };
                        let current = state.documents.lock().unwrap_or_else(|e| e.into_inner()).get(&uri).is_some_and(|document| document.present && document.version == version);
                        if !current { continue; }
                        if opened.contains_key(&uri) {
                            write_frame(&mut stdin, json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{"textDocument":{"uri":uri,"version":version},"contentChanges":[{"text":text}]}})).await?;
                        } else {
                            write_frame(&mut stdin, json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":uri,"languageId":config.language_id,"version":version,"text":text}}})).await?;
                        }
                        opened.insert(uri.clone(), version);
                        write_frame(&mut stdin, json!({"jsonrpc":"2.0","method":"textDocument/didSave","params":{"textDocument":{"uri":uri},"text":text}})).await?;
                        if pull {
                            pending.retain(|_, (pending_uri, _, sent)| pending_uri != &uri && sent.elapsed() < INITIALIZE_TIMEOUT);
                            if pending.len() < MAX_DOCUMENTS {
                                let id = request_id;
                                request_id = request_id.checked_add(1).ok_or("LSP request id exhausted")?;
                                pending.insert(id, (uri.clone(), version, Instant::now()));
                                write_frame(&mut stdin, json!({"jsonrpc":"2.0","id":id,"method":"textDocument/diagnostic","params":{"textDocument":{"uri":uri}}})).await?;
                            }
                        }
                    }
                    message = messages.recv() => {
                        let message = message.ok_or("LSP server closed")??;
                        if message.get("method").and_then(Value::as_str) == Some("textDocument/publishDiagnostics") {
                            if let Some(params) = message.get("params") {
                                if let Some(uri) = params.get("uri").and_then(Value::as_str) {
                                    let version = params.get("version").and_then(Value::as_i64).and_then(|value| i32::try_from(value).ok());
                                    publish(&state, uri, version, params.get("diagnostics"));
                                }
                            }
                        } else if message.get("method").is_some() {
                            answer_server_request(&mut stdin, &message, &root_uri).await?;
                        } else if let Some((uri, version, sent)) = message.get("id").and_then(Value::as_i64).and_then(|id| pending.remove(&id)) {
                            if sent.elapsed() < INITIALIZE_TIMEOUT && message.pointer("/result/kind").and_then(Value::as_str) == Some("full") {
                                publish(&state, &uri, Some(version), message.pointer("/result/items"));
                            }
                        }
                    }
                    _ = expiry.tick() => pending.retain(|_, (_, _, sent)| sent.elapsed() < INITIALIZE_TIMEOUT),
                    _ = &mut idle => return Ok(()),
                }
            }
        } => result,
    };
    state.close();
    if initialized {
        let _ = timeout(Duration::from_secs(1), async {
            write_frame(
                &mut stdin,
                json!({"jsonrpc":"2.0","id":-1,"method":"shutdown","params":null}),
            )
            .await?;
            while let Some(message) = messages.recv().await {
                if message?.get("id").and_then(Value::as_i64) == Some(-1) {
                    break;
                }
            }
            write_frame(&mut stdin, json!({"jsonrpc":"2.0","method":"exit"})).await?;
            while messages.recv().await.is_some_and(|message| message.is_ok()) {}
            Ok::<_, String>(())
        })
        .await;
    }
    process.kill();
    let _ = timeout(Duration::from_millis(500), process.child.wait()).await;
    result
}

fn publish(state: &Arc<SessionState>, uri: &str, version: Option<i32>, values: Option<&Value>) {
    let Some(values) = values.and_then(Value::as_array) else {
        return;
    };
    let (event, emit) = {
        let documents = state.documents.lock().unwrap_or_else(|e| e.into_inner());
        let Some(document) = documents.get(uri) else {
            return;
        };
        // shortcut: ignore ambiguous versionless pushes after edits; use versioned pushes or correlated pull requests.
        let version = match version {
            Some(version) => version,
            None if document.version == 1 => 1,
            None => return,
        };
        if version != document.version || !document.present || !state.active.load(Ordering::Acquire)
        {
            return;
        }
        let diagnostics = values
            .iter()
            .take(128)
            .filter_map(|value| serde_json::from_value::<LspDiagnostic>(value.clone()).ok())
            .map(|mut diagnostic| {
                if diagnostic.message.len() > 2048 {
                    let mut end = 2048;
                    while !diagnostic.message.is_char_boundary(end) {
                        end -= 1;
                    }
                    diagnostic.message.truncate(end);
                }
                diagnostic
            })
            .collect();
        (
            LspDiagnostics {
                owner: state.owner.clone(),
                workspace: state.workspace.clone(),
                path: document.path.clone(),
                uri: uri.into(),
                generation: state.generation,
                version,
                diagnostics,
                state: Arc::downgrade(state),
            },
            document.emit.clone(),
        )
    };
    emit(event);
}

async fn answer_server_request(
    stdin: &mut ChildStdin,
    message: &Value,
    root_uri: &str,
) -> Result<(), String> {
    let (Some(id), Some(method)) = (
        message.get("id"),
        message.get("method").and_then(Value::as_str),
    ) else {
        return Ok(());
    };
    let result = match method {
        "workspace/configuration" => Some(Value::Array(
            message
                .pointer("/params/items")
                .and_then(Value::as_array)
                .map(|items| vec![Value::Null; items.len()])
                .unwrap_or_default(),
        )),
        "workspace/workspaceFolders" => Some(json!([{"uri":root_uri,"name":"workspace"}])),
        "window/workDoneProgress/create" => Some(Value::Null),
        _ => None,
    };
    let response = match result {
        Some(result) => json!({"jsonrpc":"2.0","id":id,"result":result}),
        None => {
            json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"Unsupported LSP client method"}})
        }
    };
    write_frame(stdin, response).await
}

async fn write_frame(stdin: &mut ChildStdin, value: Value) -> Result<(), String> {
    let bytes = serde_json::to_vec(&value).map_err(|e| e.to_string())?;
    if bytes.len() > MAX_FRAME_BYTES {
        return Err("LSP output frame exceeds limit".into());
    }
    timeout(IO_TIMEOUT, async {
        stdin
            .write_all(format!("Content-Length: {}\r\n\r\n", bytes.len()).as_bytes())
            .await?;
        stdin.write_all(&bytes).await?;
        stdin.flush().await
    })
    .await
    .map_err(|_| "LSP write timed out")?
    .map_err(|e| e.to_string())
}

async fn read_frame(reader: &mut (impl AsyncBufRead + Unpin)) -> Result<Value, String> {
    let mut header = Vec::new();
    while !header.ends_with(b"\r\n\r\n") {
        if header.len() >= MAX_HEADER_BYTES {
            return Err("LSP header exceeds limit".into());
        }
        header.push(reader.read_u8().await.map_err(|e| e.to_string())?);
    }
    let header = std::str::from_utf8(&header).map_err(|_| "Invalid LSP header")?;
    let mut length = None;
    for line in header.split("\r\n").filter(|line| !line.is_empty()) {
        let (name, value) = line.split_once(':').ok_or("Invalid LSP header field")?;
        if name.eq_ignore_ascii_case("Content-Length") {
            if length.is_some() {
                return Err("Duplicate LSP Content-Length".into());
            }
            length = Some(
                value
                    .trim()
                    .parse::<usize>()
                    .map_err(|_| "Invalid LSP Content-Length")?,
            );
        }
    }
    let length = length
        .filter(|length| *length <= MAX_FRAME_BYTES)
        .ok_or("LSP frame length missing or exceeds limit")?;
    let mut bytes = vec![0; length];
    reader
        .read_exact(&mut bytes)
        .await
        .map_err(|e| e.to_string())?;
    serde_json::from_slice(&bytes).map_err(|e| format!("Invalid LSP JSON: {e}"))
}

#[cfg(test)]
#[path = "lsp/tests.rs"]
mod tests;
