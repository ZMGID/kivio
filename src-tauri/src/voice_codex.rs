#[path = "voice_codex_mcp.rs"]
mod mcp;
#[path = "voice_codex_native.rs"]
mod native;

use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::Path,
    process::Stdio,
    sync::{Arc, Mutex},
    time::Duration,
};

use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, State};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{ChildStdin, ChildStdout},
    sync::{mpsc, oneshot, watch},
    time::timeout,
};
use tokio_util::sync::CancellationToken;

use crate::{
    external_agents::{
        registry::get_agent_def,
        spawn::{agent_cli_command, resolve_binary, AgentChild},
    },
    proc::NoConsoleWindow,
};

const INSTRUCTIONS: &str = "You are Kivio's voice orchestrator. Use the Kivio MCP tools to list and read projects and conversations, create chats with the right Kivio or external agent, send requests, and check their progress. Delegate coding and file changes to those chats; do not perform them yourself. Create independent chats in parallel. Report actions and status only when confirmed by tools. Speak briefly and conversationally in the user's language, without Markdown or reading paths aloud.";
const MAX_RPC_LINE: usize = 256 * 1024;
const MAX_PENDING: usize = 256;
const MAX_TRANSCRIPT: usize = 64 * 1024;
const MAX_ITEMS: usize = 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const CANCELLED: &str = "Codex voice session cancelled";

#[derive(Clone, Default)]
pub(crate) struct VoiceCodexService {
    session: Arc<Mutex<SessionState>>,
    start_lock: Arc<tokio::sync::Mutex<()>>,
}

#[derive(Default)]
struct SessionState {
    current: Option<SessionSlot>,
    pending: HashMap<String, (CancellationToken, bool)>,
}

#[derive(Clone)]
struct SessionSlot {
    id: String,
    cancel: CancellationToken,
    control: mpsc::Sender<Control>,
    done: watch::Receiver<Option<Result<(), String>>>,
}

enum Control {
    Mute(bool, oneshot::Sender<Result<(), String>>),
}

struct CancelStart(Option<CancellationToken>);
impl Drop for CancelStart {
    fn drop(&mut self) {
        if let Some(cancel) = self.0.take() {
            cancel.cancel();
        }
    }
}

#[tauri::command]
pub(crate) fn voice_codex_reserve(service: State<'_, VoiceCodexService>) -> Result<String, String> {
    service.reserve()
}

#[tauri::command]
pub(crate) async fn voice_codex_start(
    app: AppHandle,
    service: State<'_, VoiceCodexService>,
    session_id: String,
) -> Result<(), String> {
    let cancel = service.claim(&session_id)?;
    let (control, controls) = mpsc::channel(8);
    let (done, done_rx) = watch::channel(None);
    let (started, result) = oneshot::channel();
    let mut guard = CancelStart(Some(cancel.clone()));
    let slot = SessionSlot {
        id: session_id,
        cancel,
        control,
        done: done_rx,
    };
    let service = service.inner().clone();
    tauri::async_runtime::spawn(async move {
        run_session(app, service, slot, controls, started, done).await;
    });
    result
        .await
        .map_err(|_| "Codex voice session stopped during startup".to_string())??;
    guard.0 = None;
    Ok(())
}

#[tauri::command]
pub(crate) async fn voice_codex_stop(
    service: State<'_, VoiceCodexService>,
    session_id: String,
) -> Result<(), String> {
    service.stop(Some(&session_id)).await
}

#[tauri::command]
pub(crate) async fn voice_codex_mute(
    service: State<'_, VoiceCodexService>,
    session_id: String,
    muted: bool,
) -> Result<(), String> {
    let slot = service
        .session
        .lock()
        .map_err(|_| "Voice session lock failed")?
        .current
        .as_ref()
        .filter(|slot| slot.id == session_id)
        .cloned()
        .ok_or_else(|| "Codex voice session is not active".to_string())?;
    let (reply, result) = oneshot::channel();
    timeout(Duration::from_secs(12), async {
        slot.control
            .send(Control::Mute(muted, reply))
            .await
            .map_err(|_| "Codex voice session is not active".to_string())?;
        result
            .await
            .map_err(|_| "Codex voice session ended".to_string())?
    })
    .await
    .map_err(|_| "Codex microphone control timed out".to_string())?
}

impl VoiceCodexService {
    fn reserve(&self) -> Result<String, String> {
        let id = uuid::Uuid::new_v4().to_string();
        self.session
            .lock()
            .map_err(|_| "Voice session lock failed")?
            .pending
            .insert(id.clone(), (CancellationToken::new(), false));
        Ok(id)
    }

    fn claim(&self, id: &str) -> Result<CancellationToken, String> {
        let mut state = self
            .session
            .lock()
            .map_err(|_| "Voice session lock failed")?;
        let (cancel, claimed) = state.pending.get_mut(id).ok_or(CANCELLED)?;
        if *claimed || cancel.is_cancelled() {
            return Err(CANCELLED.into());
        }
        *claimed = true;
        Ok(cancel.clone())
    }

    async fn replace(&self, next: SessionSlot) -> Result<(), String> {
        let _start = self.start_lock.lock().await;
        let previous = {
            let mut state = self
                .session
                .lock()
                .map_err(|_| "Voice session lock failed")?;
            let (cancel, claimed) = state.pending.remove(&next.id).ok_or(CANCELLED)?;
            if !claimed || cancel.is_cancelled() || next.cancel.is_cancelled() {
                return Err(CANCELLED.into());
            }
            state.current.replace(next.clone())
        };
        if let Some(previous) = previous {
            if let Err(error) = stop_slot(previous.clone()).await {
                self.session
                    .lock()
                    .map_err(|_| "Voice session lock failed")?
                    .current = Some(previous);
                return Err(error);
            }
        }
        if next.cancel.is_cancelled() {
            self.finish(&next.id);
            return Err(CANCELLED.into());
        }
        Ok(())
    }

    pub(crate) fn cancel(&self) {
        if let Ok(mut state) = self.session.lock() {
            for (_, (cancel, _)) in state.pending.drain() {
                cancel.cancel();
            }
            if let Some(slot) = state.current.as_ref() {
                slot.cancel.cancel();
            }
        }
    }

    async fn stop(&self, id: Option<&str>) -> Result<(), String> {
        let slot = {
            let mut state = self
                .session
                .lock()
                .map_err(|_| "Voice session lock failed")?;
            if let Some(id) = id {
                if let Some((cancel, _)) = state.pending.remove(id) {
                    cancel.cancel();
                }
            } else {
                for (_, (cancel, _)) in state.pending.drain() {
                    cancel.cancel();
                }
            }
            state
                .current
                .as_ref()
                .filter(|slot| id.is_none_or(|id| id == slot.id))
                .cloned()
        };
        match slot {
            Some(slot) => stop_slot(slot).await,
            None => Ok(()),
        }
    }

    pub(crate) async fn shutdown(&self) -> Result<(), String> {
        self.stop(None).await
    }

    fn finish(&self, id: &str) {
        if let Ok(mut state) = self.session.lock() {
            if state.current.as_ref().is_some_and(|slot| slot.id == id) {
                state.current = None;
            }
        }
    }
}

#[cfg(test)]
mod session_tests {
    use super::*;

    fn slot(id: &str) -> (SessionSlot, watch::Sender<Option<Result<(), String>>>) {
        let (control, _) = mpsc::channel(1);
        let (done, done_rx) = watch::channel(None);
        (
            SessionSlot {
                id: id.into(),
                cancel: CancellationToken::new(),
                control,
                done: done_rx,
            },
            done,
        )
    }

    fn reserved_slot(
        service: &VoiceCodexService,
    ) -> (SessionSlot, watch::Sender<Option<Result<(), String>>>) {
        let id = service.reserve().unwrap();
        let (mut slot, done) = slot(&id);
        slot.cancel = service.claim(&id).unwrap();
        (slot, done)
    }

    #[tokio::test]
    async fn stop_before_start_claim_prevents_a_late_start() {
        let service = VoiceCodexService::default();
        let id = service.reserve().unwrap();
        service.stop(Some(&id)).await.unwrap();
        assert!(service.claim(&id).is_err());
    }

    #[tokio::test]
    async fn stop_after_claim_before_registration_prevents_media_start() {
        let service = VoiceCodexService::default();
        let id = service.reserve().unwrap();
        let cancel = service.claim(&id).unwrap();
        let (mut pending, _done) = slot(&id);
        pending.cancel = cancel;
        service.stop(Some(&id)).await.unwrap();
        assert!(service.replace(pending).await.is_err());
        assert!(service.session.lock().unwrap().current.is_none());
    }

    #[tokio::test]
    async fn reload_cancels_starts_waiting_for_the_start_lock() {
        let service = VoiceCodexService::default();
        let id = service.reserve().unwrap();
        let cancel = service.claim(&id).unwrap();
        let (mut pending, _done) = slot(&id);
        pending.cancel = cancel;
        let lock = service.start_lock.lock().await;
        let replacement = service.replace(pending);
        tokio::pin!(replacement);
        tokio::select! {
            biased;
            result = &mut replacement => panic!("started while lock was held: {result:?}"),
            _ = tokio::task::yield_now() => {}
        }
        service.cancel();
        drop(lock);
        assert!(replacement.await.is_err());
        assert!(service.session.lock().unwrap().current.is_none());
    }

    #[tokio::test]
    async fn lost_cleanup_confirmation_blocks_replacement() {
        let service = VoiceCodexService::default();
        let old_id = service.reserve().unwrap();
        let old_cancel = service.claim(&old_id).unwrap();
        let (mut old, done) = slot(&old_id);
        old.cancel = old_cancel;
        service.replace(old).await.unwrap();
        drop(done);
        let next_id = service.reserve().unwrap();
        let next_cancel = service.claim(&next_id).unwrap();
        let (mut next, _next_done) = slot(&next_id);
        next.cancel = next_cancel;
        assert!(service.replace(next).await.is_err());
        assert_eq!(
            service.session.lock().unwrap().current.as_ref().unwrap().id,
            old_id
        );
    }

    #[tokio::test]
    async fn failed_cleanup_blocks_replacement_and_preserves_the_old_slot() {
        let service = VoiceCodexService::default();
        let (old, done) = reserved_slot(&service);
        service.replace(old.clone()).await.unwrap();
        done.send(Some(Err("cleanup failed".into()))).unwrap();
        let (next, _next_done) = reserved_slot(&service);
        assert_eq!(service.replace(next).await.unwrap_err(), "cleanup failed");
        assert_eq!(
            service.session.lock().unwrap().current.as_ref().unwrap().id,
            old.id
        );
        assert_eq!(service.shutdown().await.unwrap_err(), "cleanup failed");
    }

    #[tokio::test]
    async fn replacement_waits_for_cleanup_and_late_old_stop_cannot_cancel_it() {
        let service = VoiceCodexService::default();
        let (old, done) = reserved_slot(&service);
        service.replace(old.clone()).await.unwrap();
        let (next, _next_done) = reserved_slot(&service);
        let replacement = service.replace(next.clone());
        tokio::pin!(replacement);
        tokio::select! {
            biased;
            result = &mut replacement => panic!("started before cleanup: {result:?}"),
            _ = old.cancel.cancelled() => {}
        }
        service.finish(&old.id);
        assert_eq!(
            service.session.lock().unwrap().current.as_ref().unwrap().id,
            next.id
        );
        done.send(Some(Ok(()))).unwrap();
        replacement.await.unwrap();
        service.stop(Some(&old.id)).await.unwrap();
        assert!(!next.cancel.is_cancelled());
        service.cancel();
        assert!(next.cancel.is_cancelled());
    }

    #[tokio::test]
    async fn page_reload_cancels_the_reserved_start_while_previous_session_closes() {
        let service = VoiceCodexService::default();
        let (old, done) = reserved_slot(&service);
        service.replace(old.clone()).await.unwrap();
        let (next, _next_done) = reserved_slot(&service);
        let replacement = service.replace(next.clone());
        tokio::pin!(replacement);
        tokio::select! {
            biased;
            result = &mut replacement => panic!("started before cleanup: {result:?}"),
            _ = old.cancel.cancelled() => {}
        }
        service.cancel();
        assert!(next.cancel.is_cancelled());
        done.send(Some(Ok(()))).unwrap();
        assert!(replacement.await.is_err());
        assert!(service.session.lock().unwrap().current.is_none());
    }
}

async fn stop_slot(mut slot: SessionSlot) -> Result<(), String> {
    slot.cancel.cancel();
    timeout(Duration::from_secs(5), async {
        loop {
            if let Some(result) = slot.done.borrow_and_update().clone() {
                return result;
            }
            slot.done
                .changed()
                .await
                .map_err(|_| "Codex voice cleanup confirmation was lost".to_string())?;
        }
    })
    .await
    .map_err(|_| "Codex voice cleanup timed out".to_string())?
}

fn emit(app: &AppHandle, id: &str, mut event: Value) {
    event["sessionId"] = json!(id);
    let _ = app.emit("voice-codex-event", event);
}

async fn run_session(
    app: AppHandle,
    service: VoiceCodexService,
    slot: SessionSlot,
    mut controls: mpsc::Receiver<Control>,
    started: oneshot::Sender<Result<(), String>>,
    done: watch::Sender<Option<Result<(), String>>>,
) {
    if let Err(error) = service.replace(slot.clone()).await {
        let cleanup = if error == CANCELLED {
            Ok(())
        } else {
            Err(error.clone())
        };
        let _ = done.send(Some(cleanup));
        let _ = started.send(Err(error));
        return;
    }
    let id = slot.id;
    let cancel = slot.cancel;
    let mut runtime = Runtime::default();
    let setup = tokio::select! {
        biased;
        _ = cancel.cancelled() => Err(CANCELLED.to_string()),
        result = runtime.start(&app, &id, &cancel) => result,
    };
    match setup {
        Ok(early) => {
            emit(&app, &id, json!({"type":"active"}));
            if started.send(Ok(())).is_err() {
                cancel.cancel();
            }
            let error = runtime
                .run(&app, &id, &cancel, &mut controls, early)
                .await
                .err();
            let cleanup = runtime.close().await;
            if cleanup.is_ok() {
                service.finish(&id);
            }
            let _ = done.send(Some(cleanup.clone()));
            let mut event = json!({"type":"closed"});
            if let Some(error) = cleanup.err().or(error) {
                event["error"] = json!(error);
            }
            emit(&app, &id, event);
        }
        Err(error) => {
            let cleanup = runtime.close().await;
            if cleanup.is_ok() {
                service.finish(&id);
            }
            let _ = done.send(Some(cleanup.clone()));
            let _ = started.send(Err(cleanup.err().unwrap_or(error)));
        }
    }
}

#[derive(Default)]
struct Runtime {
    rpc: Option<Rpc>,
    host: Option<native::NativeHost>,
    mcp: Option<mcp::VoiceMcp>,
    cwd: Option<tempfile::TempDir>,
    thread: Option<String>,
    server_started: bool,
}

impl Runtime {
    async fn start(
        &mut self,
        app: &AppHandle,
        session: &str,
        cancel: &CancellationToken,
    ) -> Result<VecDeque<Value>, String> {
        let def = get_agent_def("codex")
            .ok_or_else(|| "unavailable: Codex is not registered".to_string())?;
        let resolved = timeout(Duration::from_secs(15), resolve_binary(def))
            .await
            .ok()
            .flatten();
        let (binary, helper) =
            native::discover(resolved.as_deref().unwrap_or_else(|| Path::new("")))?;
        let cwd = tempfile::Builder::new()
            .prefix("kivio-voice-")
            .tempdir()
            .map_err(|_| "Could not create a private voice working directory".to_string())?;
        let cwd_path = crate::utils::strip_windows_verbatim_prefix(cwd.path().to_path_buf());
        let cwd_string = cwd_path.to_string_lossy().into_owned();
        self.cwd = Some(cwd);
        self.rpc = Some(Rpc::spawn(def, &binary, &cwd_path)?);
        let rpc = self.rpc.as_mut().unwrap();
        rpc.request(
            "initialize",
            json!({
                "clientInfo":{"name":"kivio","title":"Kivio","version":env!("CARGO_PKG_VERSION")},
                "capabilities":{"experimentalApi":true}
            }),
            REQUEST_TIMEOUT,
        )
        .await?;
        rpc.notify("initialized", json!({})).await?;
        let account = rpc
            .request(
                "account/read",
                json!({"refreshToken":false}),
                REQUEST_TIMEOUT,
            )
            .await?;
        if account["account"]["type"] != "chatgpt" {
            return Err("unavailable: Codex must be signed in with a ChatGPT subscription".into());
        }
        // The first account/read announces the current account, not a later identity change.
        rpc.pending.clear();
        let config = rpc
            .request(
                "config/read",
                json!({"includeLayers":false,"cwd":cwd_string}),
                REQUEST_TIMEOUT,
            )
            .await?;
        if !config["config"].is_object() {
            return Err("Codex returned an invalid configuration".into());
        }
        if !official_config(&config["config"]) {
            return Err("unavailable: Codex voice requires the official OpenAI provider and realtime endpoints".into());
        }
        let voices = match rpc
            .request("thread/realtime/listVoices", json!({}), REQUEST_TIMEOUT)
            .await
        {
            Err(error) if error == "Codex thread/realtime/listVoices request was rejected" => {
                return Err(
                    "unavailable: this Codex app-server does not support subscription voice".into(),
                );
            }
            result => result?,
        };
        if !voices["voices"]["v1"].is_array() {
            return Err(
                "unavailable: this Codex app-server does not expose subscription voices".into(),
            );
        }
        let server = mcp::start(app.clone(), session.to_string()).await?;
        let thread_params = thread_params(
            &cwd_string,
            &config["config"],
            &server.url,
            &server.authorization,
        )?;
        self.mcp = Some(server);
        let thread = rpc
            .request("thread/start", thread_params, REQUEST_TIMEOUT)
            .await?;
        if thread["modelProvider"] != "openai" {
            return Err(
                "unavailable: the Codex thread is not using the official OpenAI provider".into(),
            );
        }
        let thread_id = thread["thread"]["id"]
            .as_str()
            .filter(|id| !id.is_empty() && id.len() <= 256)
            .ok_or_else(|| "Codex did not return a valid voice thread".to_string())?
            .to_owned();
        self.thread = Some(thread_id.clone());
        self.host = Some(native::NativeHost::open(&helper).await?);
        let offer = self.host.as_mut().unwrap().offer().await?;
        self.server_started = true;
        rpc.request(
            "thread/realtime/start",
            json!({
                "threadId":thread_id,"transport":{"type":"webrtc","sdp":offer},"version":"v3",
                "outputModality":"audio","realtimeSessionId":session,"clientManagedHandoffs":false,
                "includeStartupContext":true,"realtimeStartInstructions":INSTRUCTIONS,
                "initialItems":[{"role":"developer","text":INSTRUCTIONS}]
            }),
            REQUEST_TIMEOUT,
        )
        .await?;
        let mut early = VecDeque::new();
        let mut accepted = false;
        let mut answer = None;
        timeout(REQUEST_TIMEOUT, async {
            while !accepted || answer.is_none() {
                let event = rpc.next_event().await?;
                let method = event["method"].as_str().unwrap_or("");
                let params = &event["params"];
                if method == "account/updated" {
                    return Err("Codex account changed during voice startup".to_string());
                }
                if params["threadId"] != thread_id {
                    continue;
                }
                match method {
                    "thread/realtime/started" => {
                        if !accepted_start(params, session) {
                            return Err("Codex returned an unexpected voice session".to_string());
                        }
                        accepted = true;
                    }
                    "thread/realtime/sdp" => {
                        let sdp = params["sdp"]
                            .as_str()
                            .filter(|s| !s.is_empty() && s.len() <= 64 * 1024)
                            .ok_or_else(|| "Codex returned an invalid voice answer".to_string())?;
                        answer = Some(sdp.to_owned());
                    }
                    "thread/realtime/closed" | "thread/realtime/error" => {
                        return Err("Codex voice disconnected during startup".to_string());
                    }
                    _ => {
                        if early.len() >= MAX_PENDING {
                            return Err("Codex voice event queue overflow".into());
                        }
                        early.push_back(event);
                    }
                }
            }
            Ok::<(), String>(())
        })
        .await
        .map_err(|_| "Codex voice signaling timed out".to_string())??;
        let host = self.host.as_mut().unwrap();
        host.answer(answer.as_deref().unwrap(), cancel).await?;
        host.controls(false).await?;
        Ok(early)
    }

    async fn run(
        &mut self,
        app: &AppHandle,
        session: &str,
        cancel: &CancellationToken,
        controls: &mut mpsc::Receiver<Control>,
        mut early: VecDeque<Value>,
    ) -> Result<(), String> {
        let thread = self.thread.as_ref().unwrap();
        let rpc = self.rpc.as_mut().unwrap();
        let host = self.host.as_mut().unwrap();
        let mut captions = Captions::default();
        let mut meter = tokio::time::interval(Duration::from_millis(100));
        meter.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            if cancel.is_cancelled() {
                return Ok(());
            }
            if let Some(event) = early.pop_front() {
                process_event(app, session, thread, &mut captions, event)?;
                continue;
            }
            let control = tokio::select! {
                biased;
                _ = cancel.cancelled() => return Ok(()),
                control = controls.recv() => match control {
                    Some(control) => Some(control),
                    None => return Ok(()),
                },
                _ = meter.tick() => None,
                event = rpc.next_event() => {
                    process_event(app, session, thread, &mut captions, event?)?;
                    continue;
                }
            };
            let operation = async {
                match control {
                    Some(Control::Mute(muted, reply)) => {
                        let result = host.controls(muted).await;
                        let error = result.as_ref().err().cloned();
                        let _ = reply.send(result);
                        if let Some(error) = error {
                            return Err(error);
                        }
                        Ok(None)
                    }
                    None => host.levels().await.map(Some),
                }
            };
            tokio::pin!(operation);
            let levels = loop {
                tokio::select! {
                    biased;
                    _ = cancel.cancelled() => return Ok(()),
                    result = &mut operation => break result?,
                    event = rpc.next_event() => process_event(app, session, thread, &mut captions, event?)?,
                }
            };
            // Keep polls spaced even after a slow helper response or a delayed timer.
            meter.reset();
            if let Some((mic, speaker)) = levels {
                emit(
                    app,
                    session,
                    json!({"type":"levels","mic":mic,"speaker":speaker}),
                );
            }
        }
    }

    async fn close(&mut self) -> Result<(), String> {
        let cleanup = match self.host.as_mut() {
            Some(host) => host.shutdown().await,
            None => Ok(()),
        };
        self.host.take();
        self.mcp.take();
        if self.server_started {
            if let (Some(rpc), Some(thread)) = (self.rpc.as_mut(), self.thread.as_ref()) {
                let _ = rpc
                    .request(
                        "thread/realtime/stop",
                        json!({"threadId":thread}),
                        Duration::from_secs(2),
                    )
                    .await;
            }
        }
        self.rpc.take();
        self.cwd.take();
        cleanup
    }
}

fn accepted_start(params: &Value, session: &str) -> bool {
    params["version"] == "v3"
        && (params["realtimeSessionId"].is_null() || params["realtimeSessionId"] == session)
}

fn official_config(config: &Value) -> bool {
    config.is_object()
        && !config["model_provider"]
            .as_str()
            .is_some_and(|provider| provider != "openai")
        && !config["model_providers"]["openai"].is_object()
        && [
            "experimental_realtime_ws_base_url",
            "experimental_realtime_webrtc_call_base_url",
            "experimental_realtime_ws_model",
        ]
        .iter()
        .all(|key| config[*key].is_null())
}

fn thread_params(
    cwd: &str,
    config: &Value,
    url: &str,
    authorization: &str,
) -> Result<Value, String> {
    let mut overrides = json!({
        "project_doc_max_bytes":0,"web_search":"disabled",
        "features.shell_tool":false,"features.apply_patch_freeform":false,
        "features.apps":false,"features.browser_use":false,"features.computer_use":false,
        "features.js_repl":false,"features.image_generation":false,"features.memories":false,
        "features.multi_agent":false,"features.multi_agent_v2":false,"agents.enabled":false,
        "mcp_optional_startup_grace_ms":10000
    });
    let map = overrides.as_object_mut().unwrap();
    if let Some(servers) = config["mcp_servers"].as_object() {
        for name in servers.keys() {
            if name.is_empty()
                || !name
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
            {
                return Err("Cannot safely disable a configured Codex MCP server".into());
            }
            map.insert(format!("mcp_servers.{name}.enabled"), json!(false));
        }
    }
    let name = format!("kivio_voice_{}", uuid::Uuid::new_v4().simple());
    map.insert(
        format!("mcp_servers.{name}"),
        json!({
            "url":url,"http_headers":{"Authorization":authorization},
            "default_tools_approval_mode":"approve","tool_timeout_sec":630
        }),
    );
    Ok(json!({
        "cwd":cwd,"ephemeral":true,"sandbox":"read-only","approvalPolicy":"never",
        "baseInstructions":INSTRUCTIONS,"developerInstructions":INSTRUCTIONS,"config":overrides
    }))
}

// Own the child and its process group together with the bounded, cancellation-safe line reader.
struct Rpc {
    _child: AgentChild,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    line: Vec<u8>,
    pending: VecDeque<Value>,
    next_id: u64,
}

impl Rpc {
    fn spawn(
        def: &crate::external_agents::types::RuntimeAgentDef,
        binary: &Path,
        cwd: &Path,
    ) -> Result<Self, String> {
        let mut command = agent_cli_command(def, binary);
        // The session bearer must never be sent through a configured HTTP proxy.
        let mut no_proxy = String::new();
        for name in ["NO_PROXY", "no_proxy"] {
            if let Ok(value) = std::env::var(name) {
                no_proxy.push_str(&value);
                no_proxy.push(',');
            }
        }
        for (key, value) in command.as_std().get_envs() {
            if key.to_string_lossy().eq_ignore_ascii_case("no_proxy") {
                if let Some(value) = value {
                    no_proxy.push_str(&value.to_string_lossy());
                    no_proxy.push(',');
                }
            }
        }
        no_proxy.push_str("127.0.0.1,localhost,::1");
        command
            .env("NO_PROXY", &no_proxy)
            .env("no_proxy", &no_proxy);
        let mut child = command
            .arg("app-server")
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .no_console_window()
            .kill_on_drop(true)
            .spawn()
            .map(AgentChild::from)
            .map_err(|_| "Could not start the Codex app-server".to_string())?;
        let input = child
            .stdin
            .take()
            .ok_or_else(|| "Codex stdin is unavailable".to_string())?;
        let output = child
            .stdout
            .take()
            .ok_or_else(|| "Codex stdout is unavailable".to_string())?;
        Ok(Self {
            _child: child,
            input,
            output: BufReader::new(output),
            line: Vec::new(),
            pending: VecDeque::new(),
            next_id: 0,
        })
    }

    async fn write(&mut self, value: Value) -> Result<(), String> {
        let mut line =
            serde_json::to_vec(&value).map_err(|_| "Could not encode Codex request".to_string())?;
        if line.len() > MAX_RPC_LINE {
            return Err("Codex request is too large".into());
        }
        line.push(b'\n');
        self.input
            .write_all(&line)
            .await
            .map_err(|_| "Codex app-server connection closed".to_string())?;
        self.input
            .flush()
            .await
            .map_err(|_| "Codex app-server connection closed".to_string())
    }

    async fn notify(&mut self, method: &str, params: Value) -> Result<(), String> {
        timeout(
            REQUEST_TIMEOUT,
            self.write(json!({"jsonrpc":"2.0","method":method,"params":params})),
        )
        .await
        .map_err(|_| "Codex app-server write timed out".to_string())?
    }

    async fn frame(&mut self) -> Result<Value, String> {
        read_rpc_frame(&mut self.output, &mut self.line).await
    }

    async fn request(
        &mut self,
        method: &str,
        params: Value,
        deadline: Duration,
    ) -> Result<Value, String> {
        timeout(deadline, async {
            self.next_id += 1;
            let id = self.next_id;
            self.write(json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))
                .await?;
            loop {
                let value = self.frame().await?;
                if value.get("method").is_some() {
                    self.route_incoming(value)?;
                } else if value["id"].as_u64() == Some(id) {
                    if value.get("error").is_some() {
                        return Err(format!("Codex {method} request was rejected"));
                    }
                    return value
                        .get("result")
                        .cloned()
                        .ok_or_else(|| format!("Codex {method} returned no result"));
                }
            }
        })
        .await
        .map_err(|_| format!("Codex {method} request timed out"))?
    }

    fn route_incoming(&mut self, value: Value) -> Result<(), String> {
        if value.get("id").is_some() {
            return Err("Codex requested an unsupported voice-session approval".into());
        }
        let method = value["method"].as_str().unwrap_or("");
        if method.starts_with("thread/realtime/") || method == "account/updated" {
            if self.pending.len() >= MAX_PENDING {
                return Err("Codex voice event queue overflow".into());
            }
            self.pending.push_back(value);
        }
        Ok(())
    }

    async fn next_event(&mut self) -> Result<Value, String> {
        loop {
            if let Some(value) = self.pending.pop_front() {
                return Ok(value);
            }
            let value = self.frame().await?;
            if value.get("method").is_some() {
                self.route_incoming(value)?;
            }
            if self.pending.is_empty() {
                tokio::task::yield_now().await;
            }
        }
    }
}

async fn read_rpc_frame(
    output: &mut (impl tokio::io::AsyncBufRead + Unpin),
    line: &mut Vec<u8>,
) -> Result<Value, String> {
    loop {
        let (length, complete) = {
            let bytes = output
                .fill_buf()
                .await
                .map_err(|_| "Codex app-server read failed".to_string())?;
            if bytes.is_empty() {
                return Err("Codex app-server exited".into());
            }
            let length = bytes
                .iter()
                .position(|byte| *byte == b'\n')
                .map_or(bytes.len(), |pos| pos + 1);
            if line.len() + length > MAX_RPC_LINE {
                return Err("Codex response is too large".into());
            }
            line.extend_from_slice(&bytes[..length]);
            (length, bytes[length - 1] == b'\n')
        };
        output.consume(length);
        if complete {
            let result =
                serde_json::from_slice(line).map_err(|_| "Codex returned invalid JSON".to_string());
            line.clear();
            return result;
        }
    }
}

#[cfg(test)]
mod transport_tests {
    use super::*;

    #[test]
    fn startup_accepts_only_matching_present_session_ids() {
        assert!(accepted_start(&json!({"version":"v3"}), "session"));
        assert!(accepted_start(
            &json!({"version":"v3","realtimeSessionId":null}),
            "session"
        ));
        assert!(accepted_start(
            &json!({"version":"v3","realtimeSessionId":"session"}),
            "session"
        ));
        assert!(!accepted_start(
            &json!({"version":"v3","realtimeSessionId":"other"}),
            "session"
        ));
        assert!(!accepted_start(
            &json!({"version":"v3","realtimeSessionId":123}),
            "session"
        ));
        assert!(!accepted_start(
            &json!({"version":"v2","realtimeSessionId":"session"}),
            "session"
        ));
    }

    #[tokio::test]
    async fn partial_rpc_frames_survive_repeated_poll_cancellation() {
        let (mut writer, reader) = tokio::io::duplex(1024);
        let mut reader = BufReader::new(reader);
        let mut line = Vec::new();
        for fragment in [&b"{\"method\":\"thread/"[..], &b"realtime/item"[..]] {
            writer.write_all(fragment).await.unwrap();
            let read = read_rpc_frame(&mut reader, &mut line);
            tokio::pin!(read);
            tokio::select! {
                biased;
                result = &mut read => panic!("unexpected complete frame: {result:?}"),
                _ = tokio::task::yield_now() => {}
            }
        }
        assert_eq!(line, b"{\"method\":\"thread/realtime/item");
        writer
            .write_all(b"/completed\"}\n{\"method\":\"account/updated\"}\n")
            .await
            .unwrap();
        assert_eq!(
            read_rpc_frame(&mut reader, &mut line).await.unwrap()["method"],
            "thread/realtime/item/completed"
        );
        assert_eq!(
            read_rpc_frame(&mut reader, &mut line).await.unwrap()["method"],
            "account/updated"
        );
        assert!(line.is_empty());
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CaptionSource {
    Flat,
    Items,
}

#[derive(Default)]
struct Captions {
    source: Option<CaptionSource>,
    flat_active: HashMap<String, String>,
    next_flat_id: u64,
    roles: HashMap<String, String>,
    completed: HashSet<String>,
    order: VecDeque<String>,
}

impl Captions {
    fn flat_id(&mut self) -> String {
        self.next_flat_id += 1;
        format!("flat-{}", self.next_flat_id)
    }

    fn normalize(
        &mut self,
        session: &str,
        thread: &str,
        method: &str,
        params: &Value,
    ) -> Option<Value> {
        if params["threadId"] != thread {
            return None;
        }
        match method {
            // Ephemeral Codex threads have no canonical item history, but still emit flat transcripts.
            "thread/realtime/transcript/delta" | "thread/realtime/transcript/done" => {
                if self.source == Some(CaptionSource::Items) {
                    return None;
                }
                let role = params["role"]
                    .as_str()
                    .filter(|role| matches!(*role, "user" | "assistant"))?;
                let field = if method.ends_with("/delta") {
                    "delta"
                } else {
                    "text"
                };
                let text = params[field]
                    .as_str()
                    .filter(|text| text.len() <= MAX_TRANSCRIPT)?;
                if text.is_empty() {
                    return None;
                }
                self.source = Some(CaptionSource::Flat);
                if field == "delta" {
                    let id = match self.flat_active.get(role) {
                        Some(id) => id.clone(),
                        None => {
                            let id = self.flat_id();
                            self.flat_active.insert(role.to_owned(), id.clone());
                            id
                        }
                    };
                    Some(json!({"type":"partial","role":role,"itemId":id,"delta":text}))
                } else {
                    // shortcut: flat replays cannot be distinguished from repeated speech, deduplicate when Codex exposes utterance identity.
                    let id = self
                        .flat_active
                        .remove(role)
                        .unwrap_or_else(|| self.flat_id());
                    Some(
                        json!({"type":"final","role":role,"itemId":id,"text":text,"preserveLonger":true}),
                    )
                }
            }
            "thread/realtime/itemAdded" | "thread/realtime/item/started" => {
                if self.source == Some(CaptionSource::Flat) {
                    return None;
                }
                let raw = &params["item"];
                let item = if raw["item"].is_object() {
                    &raw["item"]
                } else {
                    raw
                };
                if method.ends_with("/started")
                    && (item["type"] != "transcriptSegment" || item["realtimeSessionId"] != session)
                {
                    return None;
                }
                let role = item["role"]
                    .as_str()
                    .filter(|role| matches!(*role, "user" | "assistant"))?;
                let id = item["id"]
                    .as_str()
                    .filter(|id| !id.is_empty() && id.len() <= 256)?;
                if self.roles.len() < MAX_ITEMS {
                    self.roles.insert(id.to_owned(), role.to_owned());
                }
                None
            }
            "thread/realtime/item/transcript/delta" => {
                if self.source == Some(CaptionSource::Flat) {
                    return None;
                }
                let delta = params["delta"]
                    .as_str()
                    .filter(|text| text.len() <= MAX_TRANSCRIPT)?;
                let id = params["itemId"]
                    .as_str()
                    .filter(|id| !id.is_empty() && id.len() <= 256);
                if id.is_some_and(|id| self.completed.contains(id)) {
                    return None;
                }
                self.source = Some(CaptionSource::Items);
                let role = params["role"]
                    .as_str()
                    .filter(|role| matches!(*role, "user" | "assistant"))
                    .or_else(|| id.and_then(|id| self.roles.get(id).map(String::as_str)));
                let mut event = json!({"type":"partial","delta":delta});
                if let Some(id) = id {
                    event["itemId"] = json!(id);
                }
                if let Some(role) = role {
                    event["role"] = json!(role);
                }
                Some(event)
            }
            "thread/realtime/item/completed" => {
                if self.source == Some(CaptionSource::Flat) {
                    return None;
                }
                let item = &params["item"];
                if item["type"] != "transcriptSegment" || item["realtimeSessionId"] != session {
                    return None;
                }
                let role = item["role"]
                    .as_str()
                    .filter(|role| matches!(*role, "user" | "assistant"))?;
                let id = item["id"]
                    .as_str()
                    .filter(|id| !id.is_empty() && id.len() <= 256)?;
                let text = item["text"]
                    .as_str()
                    .filter(|text| text.len() <= MAX_TRANSCRIPT)?;
                if !self.completed.insert(id.to_owned()) {
                    return None;
                }
                self.source = Some(CaptionSource::Items);
                self.roles.remove(id);
                self.order.push_back(id.to_owned());
                if self.order.len() > MAX_ITEMS {
                    if let Some(old) = self.order.pop_front() {
                        self.completed.remove(&old);
                    }
                }
                Some(json!({"type":"final","role":role,"itemId":id,"text":text}))
            }
            _ => None,
        }
    }
}

fn process_event(
    app: &AppHandle,
    session: &str,
    thread: &str,
    captions: &mut Captions,
    event: Value,
) -> Result<(), String> {
    let method = event["method"].as_str().unwrap_or("");
    if method == "account/updated" {
        return Err("Codex account changed during the voice session".into());
    }
    let params = &event["params"];
    match method {
        "thread/realtime/closed" | "thread/realtime/error" if params["threadId"] == thread => {
            Err("Codex voice session disconnected".into())
        }
        _ => {
            if let Some(event) = captions.normalize(session, thread, method, params) {
                emit(app, session, event);
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_custom_provider_and_realtime_endpoints() {
        assert!(official_config(&json!({})));
        assert!(official_config(&json!({"model_provider":"openai"})));
        assert!(!official_config(&json!({"model_provider":"proxy"})));
        assert!(!official_config(&json!({"model_providers":{"openai":{}}})));
        assert!(!official_config(
            &json!({"experimental_realtime_webrtc_call_base_url":"http://example"})
        ));
    }

    #[test]
    fn voice_thread_disables_user_servers_and_direct_tools() {
        let params = thread_params(
            "/tmp",
            &json!({"mcp_servers":{"personal":{"command":"sh"}}}),
            "http://127.0.0.1:1/mcp",
            "Bearer secret",
        )
        .unwrap();
        assert_eq!(params["sandbox"], "read-only");
        assert_eq!(params["ephemeral"], true);
        assert_eq!(params["config"]["mcp_servers.personal.enabled"], false);
        assert_eq!(params["config"]["features.shell_tool"], false);
        assert!(thread_params("/tmp", &json!({"mcp_servers":{"a.b":{}}}), "url", "token").is_err());
    }

    fn received(captions: &mut Captions, event: Value) -> Option<Value> {
        captions.normalize(
            "session",
            "thread",
            event["method"].as_str()?,
            &event["params"],
        )
    }

    #[test]
    fn ephemeral_v3_flat_transcripts_stream_and_finalize_both_speakers() {
        // Payloads follow Codex 0.164's ThreadRealtimeTranscript{Delta,Done}Notification schemas.
        let mut captions = Captions::default();
        let user = received(&mut captions, json!({"method":"thread/realtime/transcript/delta","params":{"threadId":"thread","role":"user","delta":"What can you do?"}})).unwrap();
        let assistant = received(&mut captions, json!({"method":"thread/realtime/transcript/delta","params":{"threadId":"thread","role":"assistant","delta":"A bunch "}})).unwrap();
        let next = received(&mut captions, json!({"method":"thread/realtime/transcript/delta","params":{"threadId":"thread","role":"assistant","delta":"of stuff"}})).unwrap();
        assert_eq!(
            user,
            json!({"type":"partial","role":"user","itemId":"flat-1","delta":"What can you do?"})
        );
        assert_eq!(
            assistant,
            json!({"type":"partial","role":"assistant","itemId":"flat-2","delta":"A bunch "})
        );
        assert_eq!(
            next,
            json!({"type":"partial","role":"assistant","itemId":"flat-2","delta":"of stuff"})
        );
        let user_done = received(&mut captions, json!({"method":"thread/realtime/transcript/done","params":{"threadId":"thread","role":"user","text":"What can you do?"}})).unwrap();
        let assistant_done = received(&mut captions, json!({"method":"thread/realtime/transcript/done","params":{"threadId":"thread","role":"assistant","text":"A bunch of stuff, including coding."}})).unwrap();
        assert_eq!(
            user_done,
            json!({"type":"final","role":"user","itemId":"flat-1","text":"What can you do?","preserveLonger":true})
        );
        assert_eq!(
            assistant_done,
            json!({"type":"final","role":"assistant","itemId":"flat-2","text":"A bunch of stuff, including coding.","preserveLonger":true})
        );
        let final_only = received(&mut captions, json!({"method":"thread/realtime/transcript/done","params":{"threadId":"thread","role":"user","text":"One more question"}})).unwrap();
        assert_eq!(
            final_only,
            json!({"type":"final","role":"user","itemId":"flat-3","text":"One more question","preserveLonger":true})
        );
        assert!(received(&mut captions, json!({"method":"thread/realtime/item/completed","params":{"threadId":"thread","item":{"id":"canonical","realtimeSessionId":"session","type":"transcriptSegment","role":"user","text":"One more question"}}})).is_none());
    }

    #[test]
    fn captions_isolate_threads_and_canonical_items_by_session_and_identity() {
        let mut captions = Captions::default();
        assert!(received(&mut captions, json!({"method":"thread/realtime/transcript/delta","params":{"threadId":"other","role":"user","delta":"foreign"}})).is_none());
        assert!(received(&mut captions, json!({"method":"thread/realtime/item/completed","params":{"threadId":"thread","item":{"id":"other","realtimeSessionId":"other","type":"transcriptSegment","role":"user","text":"foreign"}}})).is_none());
        let partial = received(&mut captions, json!({"method":"thread/realtime/item/transcript/delta","params":{"threadId":"thread","itemId":"i","delta":"Hi"}})).unwrap();
        assert_eq!(partial["itemId"], "i");
        assert!(partial.get("role").is_none());
        let _ = received(
            &mut captions,
            json!({"method":"thread/realtime/item/started","params":{"threadId":"thread","item":{"type":"transcriptSegment","realtimeSessionId":"session","id":"i","role":"user","text":""}}}),
        );
        let with_role = received(&mut captions, json!({"method":"thread/realtime/item/transcript/delta","params":{"threadId":"thread","itemId":"i","delta":" there"}})).unwrap();
        assert_eq!(with_role["role"], "user");
        let final_item = json!({"method":"thread/realtime/item/completed","params":{"threadId":"thread","item":{"type":"transcriptSegment","realtimeSessionId":"session","id":"i","role":"user","text":"Hi there"}}});
        assert_eq!(
            received(&mut captions, final_item.clone()).unwrap()["role"],
            "user"
        );
        assert!(received(&mut captions, final_item).is_none());
        assert!(received(&mut captions, json!({"method":"thread/realtime/item/transcript/delta","params":{"threadId":"thread","itemId":"i","delta":" late"}})).is_none());
        assert!(received(&mut captions, json!({"method":"thread/realtime/transcript/done","params":{"threadId":"thread","role":"user","text":"Hi there"}})).is_none());
        let final_only = received(&mut captions, json!({"method":"thread/realtime/item/completed","params":{"threadId":"thread","item":{"type":"transcriptSegment","realtimeSessionId":"session","id":"j","role":"assistant","text":"Hello"}}})).unwrap();
        assert_eq!(final_only["text"], "Hello");
    }
}
