//! Momo owns only its native window and a small projection of accepted run events.
//! Business execution and approval decisions remain with the existing chat owners.
mod behavior;
mod companion;
#[cfg(target_os = "macos")]
#[path = "macos.rs"]
mod platform;
#[cfg(target_os = "windows")]
#[path = "windows.rs"]
mod platform;
mod sim;
mod visual;

use crate::chat::protocol::{ChatProtocolEvent, ChatRunEvent, ChatRunEventEnvelope};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter, Manager};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub struct Position {
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mood {
    Idle,
    Thinking,
    Searching,
    Working,
    Speaking,
    Waiting,
    Done,
    Error,
}

pub enum NativeAction {
    Poke,
    OpenChat,
    Hide,
    PositionChanged(Position),
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct Preferences {
    enabled: bool,
    position: Option<Position>,
}

struct ActiveRun {
    conversation: String,
    seq: u64,
    phase: Mood,
    session_consent: bool,
    approvals: HashMap<String, u64>,
    questions: HashSet<String>,
}

impl ActiveRun {
    fn waiting(&self) -> bool {
        self.session_consent || !self.approvals.is_empty() || !self.questions.is_empty()
    }
}

#[derive(Default)]
struct Activity {
    runs: HashMap<String, ActiveRun>,
    recent: Option<(Mood, Instant, String)>,
}

impl Activity {
    fn observe(&mut self, envelope: &ChatRunEventEnvelope, now: Instant) {
        // seq=0 is explicitly a live-only event, not an accepted run transition.
        if envelope.seq == 0 {
            return;
        }
        if matches!(envelope.event, ChatRunEvent::RunStarted { .. }) {
            if !self.runs.contains_key(&envelope.run_id) {
                self.recent = None;
            }
            self.runs
                .entry(envelope.run_id.clone())
                .or_insert_with(|| ActiveRun {
                    conversation: envelope.conversation_id.clone(),
                    seq: envelope.seq,
                    phase: Mood::Thinking,
                    session_consent: false,
                    approvals: HashMap::new(),
                    questions: HashSet::new(),
                });
            return;
        }
        let Some(run) = self.runs.get_mut(&envelope.run_id) else {
            return;
        };
        // Withdrawal is emitted after releasing the protocol hub lock, so a
        // resumed tool can overtake it. Compare against this approval's sequence,
        // not the unrelated latest text/tool event; never remove a newer request.
        if let ChatRunEvent::ToolApprovalWithdrawn { tool_call_id } = &envelope.event {
            if run
                .approvals
                .get(tool_call_id)
                .is_some_and(|seq| *seq < envelope.seq)
            {
                run.approvals.remove(tool_call_id);
            }
            run.seq = run.seq.max(envelope.seq);
            return;
        }
        if envelope.seq <= run.seq {
            return;
        }
        run.seq = envelope.seq;
        match &envelope.event {
            ChatRunEvent::ReasoningDelta { .. } => run.phase = Mood::Thinking,
            ChatRunEvent::TextDelta { .. } => run.phase = Mood::Speaking,
            ChatRunEvent::ToolUpdated { tool } => {
                run.questions.remove(&tool.id);
                let name = tool.name.as_bytes();
                let search = [
                    b"search".as_slice(),
                    b"webfetch",
                    b"web_fetch",
                    b"knowledge",
                ]
                .iter()
                .any(|needle| {
                    name.windows(needle.len())
                        .any(|word| word.eq_ignore_ascii_case(needle))
                });
                run.phase = if search {
                    Mood::Searching
                } else {
                    Mood::Working
                };
            }
            ChatRunEvent::CompactionUpdated { .. } | ChatRunEvent::SubagentUpdated { .. } => {
                run.phase = Mood::Working
            }
            ChatRunEvent::SessionConsentRequested => run.session_consent = true,
            ChatRunEvent::ToolApprovalRequested { tool_call_id, .. } => {
                run.approvals.insert(tool_call_id.clone(), envelope.seq);
            }
            ChatRunEvent::UserPromptRequested { tool_call_id, .. } => {
                run.questions.insert(tool_call_id.clone());
            }
            ChatRunEvent::RunCompleted { .. }
            | ChatRunEvent::RunFailed { .. }
            | ChatRunEvent::RunCancelled { .. } => {
                let mood = match envelope.event {
                    ChatRunEvent::RunCompleted { .. } => Mood::Done,
                    ChatRunEvent::RunFailed { .. } => Mood::Error,
                    _ => Mood::Idle,
                };
                self.recent = Some((mood, now, run.conversation.clone()));
                self.runs.remove(&envelope.run_id);
            }
            _ => {}
        }
    }

    fn resolve(&mut self, run_id: &str, tool_call_id: Option<&str>) {
        if let Some(run) = self.runs.get_mut(run_id) {
            if let Some(id) = tool_call_id {
                run.questions.remove(id);
            } else {
                run.session_consent = false;
            }
        }
    }

    fn summary(&self, now: Instant) -> (Mood, usize, usize) {
        let waiting = self.runs.values().filter(|run| run.waiting()).count();
        let active = self.runs.len();
        let mood = if waiting > 0 {
            Mood::Waiting
        } else if self.runs.values().any(|run| run.phase == Mood::Searching) {
            Mood::Searching
        } else if self.runs.values().any(|run| run.phase == Mood::Working) {
            Mood::Working
        } else if self.runs.values().any(|run| run.phase == Mood::Speaking) {
            Mood::Speaking
        } else if active > 0 {
            Mood::Thinking
        } else {
            self.recent
                .as_ref()
                .filter(|(mood, at, _)| {
                    *mood == Mood::Error || now.duration_since(*at) < Duration::from_secs(3)
                })
                .map_or(Mood::Idle, |(mood, _, _)| *mood)
        };
        (mood, active, waiting)
    }

    fn conversation(&self, now: Instant) -> Option<&str> {
        // Stable selection keeps repeated clicks on one pending/active task.
        self.runs
            .values()
            .filter(|run| run.waiting())
            .map(|run| run.conversation.as_str())
            .min()
            .or_else(|| {
                self.runs
                    .values()
                    .map(|run| run.conversation.as_str())
                    .min()
            })
            .or_else(|| {
                self.recent
                    .as_ref()
                    .filter(|(mood, at, _)| {
                        *mood == Mood::Error || now.duration_since(*at) < Duration::from_secs(30)
                    })
                    .map(|(_, _, conversation)| conversation.as_str())
            })
    }
}

struct Inner {
    preferences: Preferences,
    activity: Activity,
    started: Instant,
    animation: sim::BlobSim,
    behavior: behavior::DesktopBehavior,
    companion: companion::Companion,
    desktop_active: bool,
    reduced_motion: bool,
    last_status: Option<(Mood, usize, usize, bool)>,
}

struct DesktopPet {
    inner: Mutex<Inner>,
    path: PathBuf,
    epoch: AtomicU64,
    queued: AtomicBool,
    suspended: AtomicBool,
    task: Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
}

fn state(app: &AppHandle) -> Option<tauri::State<'_, Arc<DesktopPet>>> {
    app.try_state::<Arc<DesktopPet>>()
}

pub fn initialize(app: &AppHandle) -> Result<(), String> {
    let path = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("desktop-pet.json");
    let mut preferences: Preferences = match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|e| format!("Invalid desktop pet preferences: {e}"))?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Preferences::default(),
        Err(error) => return Err(format!("Read desktop pet preferences: {error}")),
    };
    if preferences
        .position
        .is_some_and(|p| !p.x.is_finite() || !p.y.is_finite())
    {
        preferences.position = None;
    }
    let restore = preferences.enabled;
    preferences.enabled = false;
    app.manage(Arc::new(DesktopPet {
        inner: Mutex::new(Inner {
            preferences,
            activity: Activity::default(),
            started: Instant::now(),
            animation: sim::BlobSim::new(false),
            behavior: behavior::DesktopBehavior::new(),
            companion: companion::Companion::new(Instant::now()),
            desktop_active: false,
            reduced_motion: false,
            last_status: None,
        }),
        path,
        epoch: AtomicU64::new(0),
        queued: AtomicBool::new(false),
        suspended: AtomicBool::new(false),
        task: Mutex::new(None),
    }));
    if restore {
        set_enabled(app, true, false)?;
    }
    Ok(())
}

pub fn enabled(app: &AppHandle) -> bool {
    state(app).is_some_and(|state| state.inner.lock().preferences.enabled)
}

#[tauri::command]
pub fn desktop_pet_get_enabled(app: AppHandle) -> Result<bool, String> {
    let state = state(&app).ok_or("Desktop pet is not initialized")?;
    let enabled = state.inner.lock().preferences.enabled;
    Ok(enabled)
}

#[tauri::command]
pub async fn desktop_pet_set_enabled(app: AppHandle, enabled: bool) -> Result<bool, String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let handle = app.clone();
    app.run_on_main_thread(move || {
        let _ = tx.send(change_visibility(&handle, enabled).map(|()| enabled));
    })
    .map_err(|error| error.to_string())?;
    rx.await.map_err(|error| error.to_string())?
}

fn notify_enabled(app: &AppHandle, enabled: bool) {
    if let Err(error) = app.emit("desktop-pet-enabled-changed", enabled) {
        eprintln!("Notify desktop pet visibility: {error}");
    }
}

fn persist(state: &DesktopPet, preferences: &Preferences) -> Result<(), String> {
    let content = serde_json::to_string(preferences).map_err(|e| e.to_string())?;
    crate::chat::storage::atomic_write(&state.path, &content, "desktop-pet")
}

/// Main UI thread only: tray actions and native callbacks share this entry.
pub fn toggle(app: &AppHandle) -> Result<(), String> {
    change_visibility(app, !enabled(app))
}

fn change_visibility(app: &AppHandle, enable: bool) -> Result<(), String> {
    set_enabled(app, enable, true)?;
    // The native surface and saved preference are authoritative; a tray refresh
    // failure must not report an already-applied visibility change as rejected.
    if let Err(error) = crate::shortcuts::setup_tray(app) {
        eprintln!("Refresh desktop pet tray state: {error}");
    }
    Ok(())
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn set_enabled(app: &AppHandle, enable: bool, save: bool) -> Result<(), String> {
    let state = state(app).ok_or("Desktop pet is not initialized")?;
    let previous = state.inner.lock().preferences.clone();
    if previous.enabled == enable {
        return Ok(());
    }
    if enable {
        let (visual, position) = {
            let mut inner = state.inner.lock();
            let now = Instant::now();
            let mood = inner.activity.summary(now).0;
            inner.started = now;
            inner.reduced_motion = platform::reduced_motion();
            inner.animation = sim::BlobSim::new(inner.reduced_motion);
            inner.behavior = behavior::DesktopBehavior::new();
            inner.companion = companion::Companion::new(now);
            inner.desktop_active = false;
            inner.animation.set_mood(mood, 0.0);
            (inner.animation.sample(0.0), inner.preferences.position)
        };
        platform::create(app, &visual, position)?;
        let mut next = previous;
        next.enabled = true;
        next.position = platform::position();
        if save {
            if let Err(error) = persist(&state, &next) {
                platform::destroy();
                return Err(error);
            }
        }
        {
            let mut inner = state.inner.lock();
            inner.preferences = next;
            inner.last_status = None;
        }
        start_animation(app, &state);
    } else {
        let mut next = previous;
        next.enabled = false;
        next.position = platform::position().or(next.position);
        if save {
            persist(&state, &next)?;
        }
        state.epoch.fetch_add(1, Ordering::Relaxed);
        if let Some(task) = state.task.lock().take() {
            task.abort();
        }
        platform::destroy();
        let mut inner = state.inner.lock();
        inner.preferences = next;
        inner.last_status = None;
        inner.companion.dismiss(Instant::now());
    }
    notify_enabled(app, enable);
    Ok(())
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn set_enabled(_app: &AppHandle, enable: bool, _save: bool) -> Result<(), String> {
    if enable {
        Err("Native desktop pet supports macOS and Windows".into())
    } else {
        Ok(())
    }
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn start_animation(app: &AppHandle, state: &Arc<DesktopPet>) {
    let epoch = state.epoch.fetch_add(1, Ordering::Relaxed) + 1;
    state.suspended.store(false, Ordering::Relaxed);
    let app = app.clone();
    let shared = Arc::clone(state);
    let task = tauri::async_runtime::spawn(async move {
        loop {
            let frame_delay = {
                let inner = shared.inner.lock();
                if inner.desktop_active {
                    16 // Pickup and pointer response stay smooth.
                } else if inner
                    .animation
                    .wants_high_fps(inner.started.elapsed().as_secs_f64() * 1000.0)
                {
                    33
                } else {
                    80
                }
            };
            // One outstanding UI update maximum: a blocked UI cannot accumulate frames.
            if shared
                .queued
                .compare_exchange(false, true, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
            {
                let ui_state = Arc::clone(&shared);
                let ui_app = app.clone();
                if app
                    .run_on_main_thread(move || {
                        if ui_state.epoch.load(Ordering::Relaxed) == epoch
                            && ui_state.inner.lock().preferences.enabled
                        {
                            draw(&ui_app, &ui_state);
                        }
                        ui_state.queued.store(false, Ordering::Relaxed);
                    })
                    .is_err()
                {
                    shared.queued.store(false, Ordering::Relaxed);
                    break;
                }
            }
            let delay = if shared.suspended.load(Ordering::Relaxed) {
                1_000
            } else {
                frame_delay
            };
            tokio::time::sleep(Duration::from_millis(delay)).await;
        }
    });
    if let Some(previous) = state.task.lock().replace(task) {
        previous.abort();
    }
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn draw(app: &AppHandle, state: &DesktopPet) {
    let suspended = platform::suspended();
    state.suspended.store(suspended, Ordering::Relaxed);
    if suspended {
        state.inner.lock().companion.dismiss(Instant::now());
        platform::set_speech(None);
        return;
    }
    let now = Instant::now();
    let mut inner = state.inner.lock();
    let english = app
        .try_state::<crate::state::AppState>()
        .and_then(|state| {
            state
                .try_settings_read()
                .ok()
                .map(|settings| settings.settings_language.as_deref() == Some("en"))
        })
        .unwrap_or_else(|| inner.last_status.is_some_and(|status| status.3));
    let (mood, active, waiting) = inner.activity.summary(now);
    let status = (mood, active, waiting, english);
    if inner.last_status != Some(status) {
        let label = match (mood, english) {
            (Mood::Idle, false) => "墨墨 · Kivio 空闲",
            (Mood::Idle, true) => "Momo · Kivio idle",
            (Mood::Thinking, false) => "墨墨 · 正在思考",
            (Mood::Thinking, true) => "Momo · Thinking",
            (Mood::Searching, false) => "墨墨 · 正在搜索",
            (Mood::Searching, true) => "Momo · Searching",
            (Mood::Working, false) => "墨墨 · 正在执行",
            (Mood::Working, true) => "Momo · Working",
            (Mood::Speaking, false) => "墨墨 · 正在回答",
            (Mood::Speaking, true) => "Momo · Answering",
            (Mood::Waiting, false) => "墨墨 · 等你确认或回答",
            (Mood::Waiting, true) => "Momo · Needs your input",
            (Mood::Done, false) => "墨墨 · 回答完成",
            (Mood::Done, true) => "Momo · Answer ready",
            (Mood::Error, false) => "墨墨 · 任务失败",
            (Mood::Error, true) => "Momo · Task failed",
        };
        platform::set_status(&format!("{label} · {active} / {waiting}"));
        inner.last_status = Some(status);
    }
    let milliseconds = now.duration_since(inner.started).as_secs_f64() * 1000.0;
    inner.animation.set_mood(mood, milliseconds);
    let environment = platform::environment();
    if environment.pressed
        || environment.dragging
        || environment
            .cursor
            .is_some_and(|p| (p[0] - 84.0).hypot(p[1] - 54.0) < 220.0)
    {
        inner.animation.wake(milliseconds);
    }
    let mut visual = inner.animation.sample(milliseconds);
    let request_usage = inner.companion.tick(
        now,
        mood,
        english,
        environment.pressed || environment.dragging,
    );
    platform::set_speech(inner.companion.text());
    let reduced_motion = inner.reduced_motion;
    let motion = inner.behavior.apply(
        &mut visual,
        mood,
        environment,
        milliseconds / 1000.0,
        reduced_motion,
    );
    inner.desktop_active = motion.active;
    drop(inner);
    if request_usage {
        request_today_usage(app);
    }
    let result = platform::move_by(motion.delta).and_then(|()| platform::update(&visual));
    if motion.persist {
        if let Some(position) = platform::position() {
            native_action(app, NativeAction::PositionChanged(position));
        }
    }
    if let Err(error) = result {
        eprintln!("Desktop pet drawing failed: {error}");
        // Stop a broken renderer instead of logging and retrying twenty times a second.
        shutdown(app);
        let _ = crate::shortcuts::setup_tray(app);
    }
}

pub fn observe_protocol(app: &AppHandle, event: &ChatProtocolEvent) {
    let ChatProtocolEvent::Run(envelope) = event else {
        return;
    };
    if let Some(state) = state(app) {
        state
            .inner
            .lock()
            .activity
            .observe(envelope, Instant::now());
    }
}

fn request_today_usage(app: &AppHandle) {
    let Some(state) = state(app) else {
        return;
    };
    let state = Arc::clone(&state);
    let revision = state.inner.lock().companion.revision;
    let epoch = state.epoch.load(Ordering::Relaxed);
    let dir = app
        .try_state::<crate::state::AppState>()
        .map(|state| state.usage_dir.clone());
    tauri::async_runtime::spawn(async move {
        let result = match dir {
            Some(dir) => {
                tauri::async_runtime::spawn_blocking(move || crate::usage::today_usage(&dir))
                    .await
                    .unwrap_or_else(|error| Err(error.to_string()))
            }
            None => Err("Usage state is unavailable".into()),
        };
        let mut inner = state.inner.lock();
        if state.epoch.load(Ordering::Relaxed) == epoch && inner.preferences.enabled {
            inner
                .companion
                .finish_usage(revision, result, Instant::now());
        }
    });
}

pub fn resolve_interaction(app: &AppHandle, run_id: &str, tool_call_id: Option<&str>) {
    if let Some(state) = state(app) {
        state.inner.lock().activity.resolve(run_id, tool_call_id);
    }
}

pub fn native_action(app: &AppHandle, action: NativeAction) {
    let Some(state) = state(app) else {
        return;
    };
    match action {
        NativeAction::Poke => {
            let mut inner = state.inner.lock();
            let milliseconds = inner.started.elapsed().as_secs_f64() * 1000.0;
            inner.animation.poke(milliseconds, None);
            let now = Instant::now();
            let mood = inner.activity.summary(now).0;
            let english = inner.last_status.is_some_and(|status| status.3);
            let request = inner.companion.poke(now, mood, english);
            drop(inner);
            if request {
                request_today_usage(app);
            }
        }
        NativeAction::OpenChat => {
            state.inner.lock().companion.dismiss(Instant::now());
            #[cfg(any(target_os = "macos", target_os = "windows"))]
            platform::set_speech(None);
            let conversation = state
                .inner
                .lock()
                .activity
                .conversation(Instant::now())
                .map(str::to_owned);
            let result = match conversation {
                Some(id) => crate::shortcuts::open_chat_conversation(app, &id),
                None => crate::shortcuts::open_chat_window(app),
            };
            if let Err(error) = result {
                eprintln!("Open chat from Momo: {error}");
            }
        }
        NativeAction::Hide => {
            if let Err(error) = change_visibility(app, false) {
                eprintln!("Hide Momo: {error}");
            }
        }
        NativeAction::PositionChanged(position) => {
            if !position.x.is_finite() || !position.y.is_finite() {
                return;
            }
            let mut inner = state.inner.lock();
            let mut next = inner.preferences.clone();
            next.position = Some(position);
            match persist(&state, &next) {
                Ok(()) => inner.preferences = next,
                Err(error) => eprintln!("Save Momo position: {error}"),
            }
        }
    }
}

pub fn shutdown(app: &AppHandle) {
    if let Some(state) = state(app) {
        state.epoch.fetch_add(1, Ordering::Relaxed);
        if let Some(task) = state.task.lock().take() {
            task.abort();
        }
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        platform::destroy();
        let was_enabled = std::mem::replace(&mut state.inner.lock().preferences.enabled, false);
        if was_enabled {
            notify_enabled(app, false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::protocol::{ChatProtocolScope, CHAT_PROTOCOL_VERSION};

    fn event(run: &str, seq: u64, event: ChatRunEvent) -> ChatRunEventEnvelope {
        ChatRunEventEnvelope {
            protocol_version: CHAT_PROTOCOL_VERSION,
            scope: ChatProtocolScope::Run,
            conversation_id: format!("conversation-{run}"),
            run_id: run.into(),
            message_id: "message".into(),
            seq,
            base_revision: 0,
            event,
        }
    }
    fn start(activity: &mut Activity, run: &str, now: Instant) {
        activity.observe(
            &event(run, 1, ChatRunEvent::RunStarted { recovery: None }),
            now,
        );
    }

    #[test]
    fn waiting_takes_priority_over_other_running_tasks_and_resolves_individually() {
        let now = Instant::now();
        let mut activity = Activity::default();
        start(&mut activity, "a", now);
        start(&mut activity, "b", now);
        activity.observe(&event("a", 2, ChatRunEvent::SessionConsentRequested), now);
        activity.observe(
            &event(
                "b",
                2,
                ChatRunEvent::TextDelta {
                    delta: "reply".into(),
                    segment: None,
                },
            ),
            now,
        );
        assert_eq!(activity.summary(now), (Mood::Waiting, 2, 1));
        activity.resolve("a", None);
        assert_eq!(activity.summary(now), (Mood::Speaking, 2, 0));
    }

    #[test]
    fn completion_of_one_run_does_not_hide_another_and_late_events_do_not_revive_it() {
        let now = Instant::now();
        let mut activity = Activity::default();
        start(&mut activity, "a", now);
        start(&mut activity, "b", now);
        activity.observe(
            &event(
                "a",
                3,
                ChatRunEvent::RunCompleted {
                    full: String::new(),
                    conversation_revision: 0,
                },
            ),
            now,
        );
        activity.observe(
            &event(
                "a",
                2,
                ChatRunEvent::ReasoningDelta {
                    delta: "late".into(),
                    segment: None,
                },
            ),
            now,
        );
        assert_eq!(activity.summary(now), (Mood::Thinking, 1, 0));
        activity.observe(
            &event(
                "b",
                2,
                ChatRunEvent::RunCancelled {
                    full: String::new(),
                    conversation_revision: 0,
                },
            ),
            now,
        );
        assert_eq!(activity.summary(now), (Mood::Idle, 0, 0));
    }

    #[test]
    fn duplicate_sequence_cannot_restore_a_withdrawn_approval() {
        let now = Instant::now();
        let mut activity = Activity::default();
        start(&mut activity, "a", now);
        let approval = event(
            "a",
            2,
            ChatRunEvent::ToolApprovalRequested {
                tool_call_id: "tool".into(),
                name: "shell".into(),
                source: "native".into(),
                server_id: None,
                target: None,
                arguments_preview: String::new(),
                sensitivity: "high".into(),
            },
        );
        activity.observe(&approval, now);
        activity.observe(
            &event(
                "a",
                3,
                ChatRunEvent::ToolApprovalWithdrawn {
                    tool_call_id: "tool".into(),
                },
            ),
            now,
        );
        activity.observe(&approval, now);
        assert_eq!(activity.summary(now), (Mood::Thinking, 1, 0));
    }

    #[test]
    fn delayed_withdrawal_clears_waiting_without_clearing_a_newer_approval() {
        let now = Instant::now();
        let mut activity = Activity::default();
        start(&mut activity, "a", now);
        let approval = |seq| {
            event(
                "a",
                seq,
                ChatRunEvent::ToolApprovalRequested {
                    tool_call_id: "tool".into(),
                    name: "shell".into(),
                    source: "native".into(),
                    server_id: None,
                    target: None,
                    arguments_preview: String::new(),
                    sensitivity: "high".into(),
                },
            )
        };
        let withdrawal = |seq| {
            event(
                "a",
                seq,
                ChatRunEvent::ToolApprovalWithdrawn {
                    tool_call_id: "tool".into(),
                },
            )
        };
        activity.observe(&approval(2), now);
        activity.observe(
            &event(
                "a",
                4,
                ChatRunEvent::TextDelta {
                    delta: "resumed".into(),
                    segment: None,
                },
            ),
            now,
        );
        activity.observe(&withdrawal(3), now);
        assert_eq!(activity.summary(now), (Mood::Speaking, 1, 0));
        activity.observe(&approval(2), now);
        assert_eq!(activity.summary(now), (Mood::Speaking, 1, 0));
        activity.observe(&approval(5), now);
        activity.observe(&withdrawal(3), now);
        assert_eq!(activity.summary(now), (Mood::Waiting, 1, 1));
        activity.observe(&withdrawal(6), now);
        assert_eq!(activity.summary(now), (Mood::Speaking, 1, 0));
    }

    #[test]
    fn open_chat_target_prefers_waiting_then_active_and_expires_recent_runs() {
        let now = Instant::now();
        let mut activity = Activity::default();
        start(&mut activity, "a", now);
        start(&mut activity, "b", now);
        activity.observe(&event("b", 2, ChatRunEvent::SessionConsentRequested), now);
        assert_eq!(activity.conversation(now), Some("conversation-b"));
        activity.resolve("b", None);
        assert_eq!(activity.conversation(now), Some("conversation-a"));
        activity.observe(
            &event(
                "b",
                3,
                ChatRunEvent::RunCompleted {
                    full: String::new(),
                    conversation_revision: 0,
                },
            ),
            now,
        );
        assert_eq!(activity.conversation(now), Some("conversation-a"));
        activity.observe(
            &event(
                "a",
                2,
                ChatRunEvent::RunCancelled {
                    full: String::new(),
                    conversation_revision: 0,
                },
            ),
            now,
        );
        assert_eq!(activity.conversation(now), Some("conversation-a"));
        assert_eq!(activity.conversation(now + Duration::from_secs(31)), None);
    }

    #[test]
    fn failure_remains_actionable_until_a_new_run_and_completion_expires() {
        let now = Instant::now();
        let later = now + Duration::from_secs(60);
        let mut activity = Activity::default();
        start(&mut activity, "a", now);
        activity.observe(
            &event(
                "a",
                2,
                ChatRunEvent::RunFailed {
                    error: "offline".into(),
                    full: String::new(),
                    conversation_revision: 0,
                },
            ),
            now,
        );
        assert_eq!(activity.summary(later), (Mood::Error, 0, 0));
        assert_eq!(activity.conversation(later), Some("conversation-a"));
        start(&mut activity, "retry", later);
        assert_eq!(activity.summary(later), (Mood::Thinking, 1, 0));
        assert_eq!(activity.conversation(later), Some("conversation-retry"));
        activity.observe(
            &event(
                "retry",
                2,
                ChatRunEvent::RunCompleted {
                    full: String::new(),
                    conversation_revision: 0,
                },
            ),
            later,
        );
        assert_eq!(activity.summary(later), (Mood::Done, 0, 0));
        assert_eq!(
            activity.summary(later + Duration::from_secs(4)),
            (Mood::Idle, 0, 0)
        );
    }
}
