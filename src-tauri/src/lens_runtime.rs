//! Shared Lens session, image identity, stream generation and freeze-frame owner.
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

const LENS_OPEN_GRACE: Duration = Duration::from_secs(2);

/// Lens-owned runtime state.
///
/// `AppState` keeps a single instance of this handle, while all transitions stay here so
/// callers cannot separately mutate busy/session/payload fields and violate their ordering.
#[derive(Debug, Default)]
pub struct LensRuntimeState {
    lifecycle: Mutex<LensLifecycleState>,
    stream_generation: AtomicU64,
    pending_selection: Mutex<Option<String>>,
    freeze_frame_image_id: Mutex<Option<String>>,
    pending_reset: Mutex<Option<String>>,
}

#[derive(Debug, Default)]
struct LensSessionState {
    busy: bool,
    open_seq: u64,
    opened_at: Option<Instant>,
}

#[derive(Debug, Default)]
struct LensLifecycleState {
    session: LensSessionState,
    images: HashMap<String, PathBuf>,
    current_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LensSessionBusy;

impl LensRuntimeState {
    pub fn register_image(&self, image_id: String, path: PathBuf) {
        self.lifecycle
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .images
            .insert(image_id, path);
    }

    pub fn register_current_image(
        &self,
        expected_session_seq: u64,
        image_id: String,
        path: PathBuf,
    ) -> Result<Option<String>, PathBuf> {
        let mut lifecycle = self.lifecycle.lock().unwrap_or_else(|e| e.into_inner());
        if !lifecycle.session.busy || lifecycle.session.open_seq != expected_session_seq {
            return Err(path);
        }
        lifecycle.images.insert(image_id.clone(), path);
        Ok(lifecycle.current_id.replace(image_id))
    }

    pub fn image_path(&self, image_id: &str) -> Option<PathBuf> {
        self.lifecycle
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .images
            .get(image_id)
            .cloned()
    }

    pub fn current_image_id(&self) -> Option<String> {
        self.lifecycle
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .current_id
            .clone()
    }

    pub fn remove_image(&self, image_id: &str) -> Option<PathBuf> {
        self.lifecycle
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .images
            .remove(image_id)
    }

    pub fn remove_image_and_clear_current(&self, image_id: &str) -> Option<PathBuf> {
        let mut lifecycle = self.lifecycle.lock().unwrap_or_else(|e| e.into_inner());
        let path = lifecycle.images.remove(image_id);
        if lifecycle.current_id.as_deref() == Some(image_id) {
            lifecycle.current_id = None;
        }
        path
    }

    pub fn begin_stream(&self) -> u64 {
        self.stream_generation.fetch_add(1, Ordering::SeqCst) + 1
    }

    pub fn cancel_stream(&self) {
        self.stream_generation.fetch_add(1, Ordering::SeqCst);
    }

    pub fn is_stream_current(&self, generation: u64) -> bool {
        self.stream_generation.load(Ordering::SeqCst) == generation
    }

    /// Acquire the single Lens session slot, repairing a stale busy marker only when no overlay
    /// is visible and the just-opened grace period has elapsed. On success this also advances the
    /// watchdog sequence and starts a new grace period as one transition.
    pub fn begin_session(&self, overlay_visible: bool) -> Result<u64, LensSessionBusy> {
        let mut lifecycle = self.lifecycle.lock().unwrap_or_else(|e| e.into_inner());
        let session = &mut lifecycle.session;
        if session.busy && !overlay_visible && !Self::open_in_grace(&session) {
            session.busy = false;
        }
        if session.busy {
            return Err(LensSessionBusy);
        }

        session.busy = true;
        session.open_seq = session.open_seq.wrapping_add(1);
        session.opened_at = Some(Instant::now());
        Ok(session.open_seq)
    }

    pub fn release_session(&self) {
        self.lifecycle
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .session
            .busy = false;
    }

    pub fn close_session_and_take_current(&self) -> Option<(String, PathBuf)> {
        let mut lifecycle = self.lifecycle.lock().unwrap_or_else(|e| e.into_inner());
        lifecycle.session.busy = false;
        let image_id = lifecycle.current_id.take()?;
        lifecycle
            .images
            .remove(&image_id)
            .map(|path| (image_id, path))
    }

    /// Probe whether Lens is active, preserving the existing opening-window semantics: a busy
    /// marker inside the grace period is retained but is not reported active until a window is
    /// visible. A stale marker outside the grace period is repaired.
    pub fn is_active_or_recover(&self, mut overlay_visible: impl FnMut() -> bool) -> bool {
        let visible = overlay_visible();
        let mut lifecycle = self.lifecycle.lock().unwrap_or_else(|e| e.into_inner());
        let session = &mut lifecycle.session;
        if session.busy {
            if visible {
                return true;
            }
            if Self::open_in_grace(&session) {
                return false;
            }
            session.busy = false;
        }
        visible
    }

    pub fn session_seq(&self) -> u64 {
        self.lifecycle
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .session
            .open_seq
    }

    pub fn is_session_active(&self, seq: u64) -> bool {
        let lifecycle = self.lifecycle.lock().unwrap_or_else(|e| e.into_inner());
        lifecycle.session.busy && lifecycle.session.open_seq == seq
    }

    pub fn is_session_current(&self, seq: u64) -> bool {
        self.session_seq() == seq
    }

    fn open_in_grace(session: &LensSessionState) -> bool {
        session
            .opened_at
            .map(|at| at.elapsed() < LENS_OPEN_GRACE)
            .unwrap_or(false)
    }

    /// Replace the selection captured for the current opening. Selection is intentionally a peek,
    /// not a take: React StrictMode and cold mounts may request it twice for one Lens session.
    pub fn replace_selection(&self, selection: Option<String>) {
        if let Ok(mut pending) = self.pending_selection.lock() {
            *pending = selection;
        }
    }

    pub fn selection(&self) -> Option<String> {
        self.pending_selection
            .lock()
            .ok()
            .and_then(|pending| pending.clone())
    }

    pub fn replace_reset_payload(&self, payload: String) {
        *self.pending_reset.lock().unwrap_or_else(|e| e.into_inner()) = Some(payload);
    }

    pub fn take_reset_payload(&self) -> Option<String> {
        self.pending_reset
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
    }

    pub fn replace_freeze_frame(&self, image_id: String) -> Option<String> {
        self.freeze_frame_image_id
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .replace(image_id)
    }

    /// Publish a freeze frame only while its opening still owns the session.
    pub fn register_freeze_frame(
        &self,
        expected_session_seq: u64,
        image_id: String,
        path: PathBuf,
    ) -> Result<Option<(String, PathBuf)>, PathBuf> {
        let mut lifecycle = self.lifecycle.lock().unwrap_or_else(|e| e.into_inner());
        if !lifecycle.session.busy || lifecycle.session.open_seq != expected_session_seq {
            return Err(path);
        }
        lifecycle.images.insert(image_id.clone(), path);
        let old = self
            .freeze_frame_image_id
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .replace(image_id);
        Ok(old.and_then(|id| lifecycle.images.remove(&id).map(|path| (id, path))))
    }

    pub fn is_current_freeze_frame(&self, image_id: &str) -> bool {
        self.freeze_frame_image_id
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_deref()
            == Some(image_id)
    }

    pub fn take_freeze_frame(&self) -> Option<String> {
        self.freeze_frame_image_id
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
    }

    pub fn take_freeze_frame_if_current(&self, image_id: &str) -> bool {
        let mut current = self
            .freeze_frame_image_id
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if current.as_deref() != Some(image_id) {
            return false;
        }
        *current = None;
        true
    }
}

#[cfg(test)]
mod runtime_state_tests {
    use super::*;
    use std::sync::{Arc, Barrier};
    use std::time::{Duration, Instant};

    fn expire_open_grace(state: &LensRuntimeState) {
        state.lifecycle.lock().unwrap().session.opened_at =
            Some(Instant::now() - Duration::from_secs(3));
    }

    #[test]
    fn session_busy_recovers_only_after_open_grace_expires() {
        let state = LensRuntimeState::default();
        assert_eq!(state.begin_session(false), Ok(1));
        assert_eq!(state.begin_session(false), Err(LensSessionBusy));

        expire_open_grace(&state);
        assert_eq!(state.begin_session(false), Ok(2));
    }

    #[test]
    fn visible_session_stays_busy_even_after_open_grace_expires() {
        let state = LensRuntimeState::default();
        assert_eq!(state.begin_session(false), Ok(1));
        expire_open_grace(&state);

        assert_eq!(state.begin_session(true), Err(LensSessionBusy));
        assert!(state.is_session_current(1));
    }

    #[test]
    fn active_probe_preserves_opening_session_and_repairs_stale_busy() {
        let state = LensRuntimeState::default();
        assert_eq!(state.begin_session(false), Ok(1));
        assert!(!state.is_active_or_recover(|| false));
        assert_eq!(state.begin_session(false), Err(LensSessionBusy));

        expire_open_grace(&state);
        assert!(!state.is_active_or_recover(|| false));
        assert_eq!(state.begin_session(false), Ok(2));
        assert!(state.is_active_or_recover(|| true));
    }

    #[test]
    fn selection_is_replaced_per_session_and_can_be_peeked_repeatedly() {
        let state = LensRuntimeState::default();
        state.replace_selection(Some("selected".to_string()));
        assert_eq!(state.selection().as_deref(), Some("selected"));
        assert_eq!(state.selection().as_deref(), Some("selected"));

        state.replace_selection(None);
        assert_eq!(state.selection(), None);
    }

    #[test]
    fn reset_payload_is_consumed_once() {
        let state = LensRuntimeState::default();
        state.replace_reset_payload("reset-1".to_string());
        state.replace_reset_payload("reset-2".to_string());

        assert_eq!(state.take_reset_payload().as_deref(), Some("reset-2"));
        assert_eq!(state.take_reset_payload(), None);
    }

    #[test]
    fn freeze_frame_replace_current_and_conditional_take_are_atomic() {
        let state = LensRuntimeState::default();
        assert_eq!(state.replace_freeze_frame("frame-1".to_string()), None);
        assert!(state.is_current_freeze_frame("frame-1"));
        assert!(!state.is_current_freeze_frame("frame-2"));

        assert_eq!(
            state.replace_freeze_frame("frame-2".to_string()).as_deref(),
            Some("frame-1")
        );
        assert!(!state.take_freeze_frame_if_current("frame-1"));
        assert!(state.take_freeze_frame_if_current("frame-2"));
        assert_eq!(state.take_freeze_frame(), None);
    }

    #[test]
    fn image_identity_and_stream_generation_stay_inside_lens_owner() {
        let state = LensRuntimeState::default();
        let first_path = PathBuf::from("first.png");
        let second_path = PathBuf::from("second.png");
        assert_eq!(state.begin_session(false), Ok(1));

        assert_eq!(
            state.register_current_image(1, "first".to_string(), first_path.clone()),
            Ok(None)
        );
        assert_eq!(state.image_path("first"), Some(first_path));
        assert_eq!(state.current_image_id().as_deref(), Some("first"));
        assert_eq!(
            state.register_current_image(1, "second".to_string(), second_path),
            Ok(Some("first".to_string()))
        );
        assert_eq!(
            state.remove_image_and_clear_current("second"),
            Some(PathBuf::from("second.png"))
        );
        assert_eq!(state.current_image_id(), None);

        let generation = state.begin_stream();
        assert!(state.is_stream_current(generation));
        state.cancel_stream();
        assert!(!state.is_stream_current(generation));
    }

    #[test]
    fn close_atomically_rejects_a_late_capture_and_removes_the_current_image() {
        let state = LensRuntimeState::default();
        assert_eq!(state.begin_session(false), Ok(1));
        assert_eq!(
            state.register_current_image(1, "current".to_string(), PathBuf::from("current.png")),
            Ok(None)
        );

        assert_eq!(
            state.close_session_and_take_current(),
            Some(("current".to_string(), PathBuf::from("current.png")))
        );
        assert_eq!(state.current_image_id(), None);
        assert_eq!(state.image_path("current"), None);
        assert_eq!(
            state.register_current_image(1, "late".to_string(), PathBuf::from("late.png")),
            Err(PathBuf::from("late.png"))
        );
        assert_eq!(state.current_image_id(), None);
        assert_eq!(state.image_path("late"), None);
    }

    #[test]
    fn reopening_rejects_a_capture_from_the_previous_session() {
        let state = LensRuntimeState::default();
        let stale_session = state.begin_session(false).expect("first session");
        assert_eq!(state.close_session_and_take_current(), None);
        let current_session = state.begin_session(false).expect("reopened session");
        assert_ne!(stale_session, current_session);

        assert_eq!(
            state.register_current_image(
                stale_session,
                "stale".to_string(),
                PathBuf::from("stale.png"),
            ),
            Err(PathBuf::from("stale.png"))
        );
        assert_eq!(state.current_image_id(), None);
        assert_eq!(state.image_path("stale"), None);

        assert_eq!(
            state.register_current_image(
                current_session,
                "current".to_string(),
                PathBuf::from("current.png"),
            ),
            Ok(None)
        );
        assert_eq!(state.current_image_id().as_deref(), Some("current"));
    }

    #[test]
    fn concurrent_session_starts_publish_busy_grace_and_sequence_atomically() {
        const CONTENDERS: usize = 16;
        let state = Arc::new(LensRuntimeState::default());
        let start = Arc::new(Barrier::new(CONTENDERS));
        let handles = (0..CONTENDERS)
            .map(|_| {
                let state = Arc::clone(&state);
                let start = Arc::clone(&start);
                std::thread::spawn(move || {
                    start.wait();
                    state.begin_session(false)
                })
            })
            .collect::<Vec<_>>();

        let results = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(state.session_seq(), 1);
        assert!(!state.is_active_or_recover(|| false));
        assert_eq!(state.begin_session(false), Err(LensSessionBusy));
    }
}
