//! `InteractiveAgentHost` — an [`AgentHost`] for the interactive TUI shell
//! (Phase 5b).
//!
//! Where the print-mode [`CliAgentHost`](crate::kivio_code::host::CliAgentHost)
//! writes straight to stdout/stderr, the interactive host runs on a background
//! tokio task and must hand its events back to the **single-threaded TUI event
//! loop** so they can be folded into [`App`](super::app::App) state and rendered
//! with the diff renderer. It does that over an [`mpsc::Sender`] of
//! [`AgentUiEvent`]s.
//!
//! ## Data flow
//!
//! ```text
//!   run_agent_loop  ──emit_*──▶  InteractiveAgentHost  ──AgentUiEvent──▶  mpsc
//!        (tokio task)                                                       │
//!                                                                           ▼
//!                                     event loop  ──App::apply_agent_event──▶ App
//! ```
//!
//! The host owns no UI state; it is a pure relay plus the cancel token.
//!
//! ## Cancellation
//!
//! The loop polls [`AgentHost::is_generation_active`] before every planning
//! round and tool round. The host answers from a shared [`AtomicBool`] that the
//! event loop flips to `false` when the user presses Esc / Ctrl+C. The loop then
//! stops at its next checkpoint and returns `Err("cancelled")`, which the event
//! loop turns into a `Done{reason:"cancelled"}`-style finalize.
//!
//! ## Approval (MVP)
//!
//! Interactive mode auto-approves the core coding tools (read/write/edit/bash/…)
//! exactly like print mode's default policy — running the CLI on one's own
//! machine is the consent — but every call still surfaces as a tool *card* in the
//! transcript, and Esc cancels the whole run. A richer per-tool y/n prompt is a
//! later phase; the host is structured so it can be added without touching the
//! loop.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;

use crate::chat::agent::execute::ToolExecutionContext;
use crate::chat::agent::host::{AgentHost, AgentHostFuture};
use crate::chat::ask_user::{AskUserPromptPayload, AskUserResponseResult};
use crate::chat::types::{ChatMessageSegment, ToolCallRecord};

/// Events the background agent task relays to the TUI event loop. Each is folded
/// into [`App`](super::app::App) state by
/// [`App::apply_agent_event`](super::app::App::apply_agent_event).
#[derive(Clone, Debug)]
pub enum AgentUiEvent {
    /// A streaming token (assistant text and/or reasoning) for the in-progress
    /// assistant message identified by `message_id`.
    StreamDelta {
        message_id: String,
        /// Visible answer text delta (may be empty when only reasoning streamed).
        delta: String,
        /// Reasoning/thinking delta, when the model streams it separately.
        reasoning: String,
    },
    /// A tool-call record update. The same `record.id` is emitted multiple times
    /// as it moves Pending → Running → Success/Error; the App upserts by id.
    ToolRecord(Box<ToolCallRecord>),
    /// The assistant message finished streaming for this turn segment.
    Done {
        message_id: String,
        /// Stream outcome: `completed` | `cancelled` | `error`.
        reason: String,
    },
}

/// Shared, cheaply-cloneable cancel token for one interactive run. The event loop
/// holds a clone and flips [`cancel`](RunCancel::cancel) on Esc/Ctrl+C; the host
/// reads it from `is_generation_active`.
#[derive(Clone)]
pub struct RunCancel {
    active: Arc<AtomicBool>,
    generation: u64,
}

impl RunCancel {
    /// Start a live run at `generation` (monotonic per submit; see [`Generations`]).
    pub fn new(generation: u64) -> Self {
        Self {
            active: Arc::new(AtomicBool::new(true)),
            generation,
        }
    }

    /// The generation this run claims; the host answers `is_generation_active`
    /// only for a matching generation.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Whether the run is still live (not cancelled).
    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::SeqCst)
    }

    /// Cancel the in-flight run; the loop stops at its next generation check.
    pub fn cancel(&self) {
        self.active.store(false, Ordering::SeqCst);
    }
}

/// Monotonic generation source: each interactive submit takes the next id so a
/// stale background task (one whose run was cancelled, then superseded) can never
/// be mistaken for the live one. Shared (`Arc`) between the event loop and the
/// run-spawning code.
#[derive(Clone, Default)]
pub struct Generations {
    counter: Arc<AtomicU64>,
}

impl Generations {
    pub fn next(&self) -> u64 {
        self.counter.fetch_add(1, Ordering::SeqCst) + 1
    }
}

/// Interactive [`AgentHost`]: relays loop events to the event loop over an mpsc
/// channel and answers cancellation from a shared [`RunCancel`].
pub struct InteractiveAgentHost {
    tx: Sender<AgentUiEvent>,
    cancel: RunCancel,
}

impl InteractiveAgentHost {
    pub fn new(tx: Sender<AgentUiEvent>, cancel: RunCancel) -> Self {
        Self { tx, cancel }
    }

    fn send(&self, event: AgentUiEvent) {
        // The receiver lives for the whole interactive session; a send error only
        // happens if the event loop already exited, in which case dropping the
        // event is correct (nothing left to render).
        let _ = self.tx.send(event);
    }
}

impl AgentHost for InteractiveAgentHost {
    fn emit_stream_delta(
        &self,
        _conversation_id: &str,
        _run_id: &str,
        message_id: &str,
        delta: &str,
        reasoning_delta: Option<&str>,
        _segment: Option<&ChatMessageSegment>,
    ) {
        if delta.is_empty() && reasoning_delta.map(str::is_empty).unwrap_or(true) {
            return;
        }
        self.send(AgentUiEvent::StreamDelta {
            message_id: message_id.to_string(),
            delta: delta.to_string(),
            reasoning: reasoning_delta.unwrap_or("").to_string(),
        });
    }

    fn emit_stream_done(
        &self,
        _conversation_id: &str,
        _run_id: &str,
        message_id: &str,
        reason: &str,
        _full: &str,
    ) {
        self.send(AgentUiEvent::Done {
            message_id: message_id.to_string(),
            reason: reason.to_string(),
        });
    }

    fn emit_tool_record(
        &self,
        _conversation_id: &str,
        _run_id: &str,
        _message_id: &str,
        record: &ToolCallRecord,
    ) {
        self.send(AgentUiEvent::ToolRecord(Box::new(record.clone())));
    }

    fn request_tool_approval<'a>(
        &'a self,
        _ctx: &'a ToolExecutionContext<'a>,
        _record: &'a ToolCallRecord,
    ) -> AgentHostFuture<'a, bool> {
        // MVP: auto-approve. The tool still surfaces as a card; Esc cancels the
        // whole run. A per-tool y/n prompt is a later phase.
        Box::pin(async move { true })
    }

    fn request_session_consent<'a>(
        &'a self,
        _ctx: &'a ToolExecutionContext<'a>,
    ) -> AgentHostFuture<'a, bool> {
        // Running the CLI on one's own machine is the consent (same as print).
        Box::pin(async move { true })
    }

    fn request_user_response<'a>(
        &'a self,
        _ctx: &'a ToolExecutionContext<'a>,
        _record: &'a ToolCallRecord,
        _prompt: AskUserPromptPayload,
    ) -> AgentHostFuture<'a, AskUserResponseResult> {
        // `ask_user` is not in the core tool set, so this is unreachable in
        // practice; resolve as cancelled (mirrors print mode / SubAgentHost).
        Box::pin(async move {
            AskUserResponseResult {
                phase: "cancelled".to_string(),
                answers: std::collections::HashMap::new(),
            }
        })
    }

    fn is_generation_active(&self, _conversation_id: &str, generation: u64) -> bool {
        generation == self.cancel.generation() && self.cancel.is_active()
    }

    fn wait_for_generation_inactive<'a>(
        &'a self,
        _conversation_id: &'a str,
        generation: u64,
    ) -> AgentHostFuture<'a, ()> {
        Box::pin(async move {
            loop {
                if generation != self.cancel.generation() || !self.cancel.is_active() {
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::types::ToolCallStatus;
    use std::sync::mpsc;

    fn ctx() -> ToolExecutionContext<'static> {
        ToolExecutionContext {
            conversation_id: "kivio-code",
            run_id: "run",
            message_id: "msg",
            generation: 1,
            round: 1,
            depth: 0,
            tool_conversation_id: "kivio-code",
            tool_call_id: "call",
        }
    }

    fn record(name: &str) -> ToolCallRecord {
        ToolCallRecord {
            id: "call_1".to_string(),
            name: name.to_string(),
            source: "native".to_string(),
            server_id: None,
            arguments: "{}".to_string(),
            status: ToolCallStatus::Running,
            result_preview: None,
            error: None,
            duration_ms: None,
            started_at: None,
            completed_at: None,
            round: 1,
            sensitive: false,
            artifacts: Vec::new(),
            trace_id: None,
            span_id: None,
            structured_content: None,
        }
    }

    #[test]
    fn generations_are_monotonic() {
        let gens = Generations::default();
        assert_eq!(gens.next(), 1);
        assert_eq!(gens.next(), 2);
        assert_eq!(gens.next(), 3);
    }

    #[test]
    fn stream_delta_is_relayed() {
        let (tx, rx) = mpsc::channel();
        let host = InteractiveAgentHost::new(tx, RunCancel::new(1));
        host.emit_stream_delta("c", "r", "m1", "hello", None, None);
        match rx.recv().unwrap() {
            AgentUiEvent::StreamDelta {
                message_id,
                delta,
                reasoning,
            } => {
                assert_eq!(message_id, "m1");
                assert_eq!(delta, "hello");
                assert!(reasoning.is_empty());
            }
            other => panic!("expected StreamDelta, got {other:?}"),
        }
    }

    #[test]
    fn empty_delta_is_dropped() {
        let (tx, rx) = mpsc::channel();
        let host = InteractiveAgentHost::new(tx, RunCancel::new(1));
        host.emit_stream_delta("c", "r", "m1", "", Some(""), None);
        assert!(rx.try_recv().is_err(), "empty delta should not be sent");
    }

    #[test]
    fn tool_record_is_relayed() {
        let (tx, rx) = mpsc::channel();
        let host = InteractiveAgentHost::new(tx, RunCancel::new(1));
        host.emit_tool_record("c", "r", "m1", &record("read"));
        match rx.recv().unwrap() {
            AgentUiEvent::ToolRecord(rec) => assert_eq!(rec.name, "read"),
            other => panic!("expected ToolRecord, got {other:?}"),
        }
    }

    #[test]
    fn done_is_relayed() {
        let (tx, rx) = mpsc::channel();
        let host = InteractiveAgentHost::new(tx, RunCancel::new(1));
        host.emit_stream_done("c", "r", "m1", "completed", "full text");
        match rx.recv().unwrap() {
            AgentUiEvent::Done { message_id, reason } => {
                assert_eq!(message_id, "m1");
                assert_eq!(reason, "completed");
            }
            other => panic!("expected Done, got {other:?}"),
        }
    }

    #[test]
    fn cancel_flips_generation_active() {
        let (tx, _rx) = mpsc::channel();
        let cancel = RunCancel::new(7);
        let host = InteractiveAgentHost::new(tx, cancel.clone());
        assert!(host.is_generation_active("c", 7));
        cancel.cancel();
        assert!(!host.is_generation_active("c", 7));
    }

    #[test]
    fn stale_generation_is_inactive() {
        let (tx, _rx) = mpsc::channel();
        let host = InteractiveAgentHost::new(tx, RunCancel::new(7));
        // A live token but for a different (superseded) generation reads inactive.
        assert!(!host.is_generation_active("c", 6));
        assert!(!host.is_generation_active("c", 8));
    }

    #[tokio::test]
    async fn approval_and_consent_are_granted() {
        let (tx, _rx) = mpsc::channel();
        let host = InteractiveAgentHost::new(tx, RunCancel::new(1));
        let c = ctx();
        assert!(host.request_session_consent(&c).await);
        assert!(host.request_tool_approval(&c, &record("write")).await);
    }
}
