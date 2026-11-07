use std::sync::Mutex;

pub use super::content::InputKind;
use super::content::{AgentToolCall, ContentBlock};
use super::history::RuntimeMessage;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AgentRunIdentity<'a> {
    pub conversation_id: &'a str,
    pub run_id: &'a str,
    pub message_id: &'a str,
    pub generation: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentRunOutcome {
    Completed,
    Cancelled,
    Failed(String),
    Dropped,
}

#[derive(Clone, Debug)]
pub struct RetryStatus {
    pub reason: String,
    pub attempt: u32,
    pub max_retries: u32,
    pub retry_at_ms: u64,
}

#[derive(Debug)]
pub struct CompactionFact<'a> {
    pub id: &'a str,
    pub covered_through: &'a str,
    pub history_tail: Option<&'a str>,
    pub tokens_before: usize,
    pub tokens_after: usize,
    pub summary: &'a str,
    pub trigger: &'a str,
    pub created_at: i64,
}

#[derive(Debug)]
pub enum AgentEventKind<'a> {
    RunStarted,
    StepStarted {
        round: u32,
    },
    MessageStarted {
        round: u32,
    },
    Content {
        delta: &'a str,
        reasoning_delta: Option<&'a str>,
        block: Option<&'a ContentBlock>,
    },
    SearchUpdated {
        block: &'a ContentBlock,
        announce: bool,
    },
    ToolUpdated(&'a AgentToolCall),
    ToolStarted {
        round: u32,
        call_id: &'a str,
        name: &'a str,
    },
    ToolEnded {
        round: u32,
        call_id: &'a str,
        name: &'a str,
        cancelled: bool,
    },
    AttemptDiscarded {
        text_chars: u32,
        reasoning_chars: u32,
        block_ids: Vec<u32>,
        tool_ids: Vec<String>,
    },
    Status {
        note: Option<String>,
    },
    Retry(RetryStatus),
    ContextPrepared {
        provider_id: &'a str,
        model: &'a str,
        messages: &'a [RuntimeMessage],
        tools: &'a [crate::model::ModelTool],
    },
    ContextCache {
        cache: Option<(u64, u64)>,
    },
    ContextUnreported,
    ContextUsage {
        used_tokens: u64,
        source: Option<&'a str>,
        window: Option<u64>,
        cache: Option<(u64, u64)>,
    },
    Compaction {
        phase: &'a str,
        trigger: Option<&'a str>,
        boundary: Option<CompactionFact<'a>>,
    },
    MessageEnded {
        round: u32,
    },
    StepEnded {
        round: u32,
    },
    RunEnded {
        outcome: AgentRunOutcome,
    },
}

pub struct AgentEvent<'a> {
    pub identity: AgentRunIdentity<'a>,
    pub invocation_id: u64,
    pub sequence: u64,
    pub kind: AgentEventKind<'a>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventDelivery {
    Accepted,
    Closed,
}

/// Direct, serialized observation with no backlog. Slow callbacks slow the producer.
/// Closed detaches observation; approvals, persistence and cleanup remain host operations.
pub trait AgentEventSink: Send + Sync {
    fn on_event(&self, event: &AgentEvent<'_>) -> EventDelivery;
}

impl<F> AgentEventSink for F
where
    F: Fn(&AgentEvent<'_>) -> EventDelivery + Send + Sync,
{
    fn on_event(&self, event: &AgentEvent<'_>) -> EventDelivery {
        self(event)
    }
}

struct EventState {
    sequence: u64,
    closed: bool,
    ended: bool,
}

pub(crate) struct AgentEvents<'a> {
    pub(crate) identity: AgentRunIdentity<'a>,
    invocation_id: u64,
    sink: &'a dyn AgentEventSink,
    hooks: Option<&'a dyn AgentEventSink>,
    state: Mutex<EventState>,
}

impl<'a> AgentEvents<'a> {
    pub(crate) fn new(
        identity: AgentRunIdentity<'a>,
        sink: &'a dyn AgentEventSink,
        hooks: Option<&'a dyn AgentEventSink>,
        invocation_id: u64,
    ) -> Self {
        Self {
            identity,
            invocation_id,
            sink,
            hooks,
            state: Mutex::new(EventState {
                sequence: 0,
                closed: false,
                ended: false,
            }),
        }
    }

    pub(crate) fn emit(&self, kind: AgentEventKind<'_>) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if state.ended {
            return;
        }
        state.ended = matches!(kind, AgentEventKind::RunEnded { .. });
        state.sequence += 1;
        let event = AgentEvent {
            identity: self.identity,
            invocation_id: self.invocation_id,
            sequence: state.sequence,
            kind,
        };
        if let Some(hooks) = self.hooks {
            hooks.on_event(&event);
        }
        if !state.closed {
            state.closed = self.sink.on_event(&event) == EventDelivery::Closed;
        }
    }
}

pub(crate) struct ToolEventGuard<'a> {
    events: &'a AgentEvents<'a>,
    round: u32,
    call_id: &'a str,
    name: &'a str,
    pub(crate) cancelled: bool,
}

impl<'a> ToolEventGuard<'a> {
    pub(crate) fn new(
        events: &'a AgentEvents<'a>,
        round: u32,
        call_id: &'a str,
        name: &'a str,
    ) -> Self {
        events.emit(AgentEventKind::ToolStarted {
            round,
            call_id,
            name,
        });
        Self {
            events,
            round,
            call_id,
            name,
            cancelled: true,
        }
    }
}

impl Drop for ToolEventGuard<'_> {
    fn drop(&mut self) {
        self.events.emit(AgentEventKind::ToolEnded {
            round: self.round,
            call_id: self.call_id,
            name: self.name,
            cancelled: self.cancelled,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc,
    };
    fn identity() -> AgentRunIdentity<'static> {
        AgentRunIdentity {
            conversation_id: "session",
            run_id: "run",
            message_id: "message",
            generation: 1,
        }
    }
    #[test]
    fn closed_observer_does_not_close_other_consumers_and_terminal_rejects_late_facts() {
        let calls = AtomicUsize::new(0);
        let closed = |_: &AgentEvent<'_>| {
            calls.fetch_add(1, Ordering::SeqCst);
            EventDelivery::Closed
        };
        let recorded = Mutex::new(Vec::new());
        let observer = |event: &AgentEvent<'_>| {
            recorded
                .lock()
                .unwrap()
                .push((event.invocation_id, event.sequence));
            EventDelivery::Accepted
        };
        let events = AgentEvents::new(identity(), &closed, Some(&observer), 9);
        events.emit(AgentEventKind::RunStarted);
        events.emit(AgentEventKind::Status { note: None });
        events.emit(AgentEventKind::RunEnded {
            outcome: AgentRunOutcome::Completed,
        });
        events.emit(AgentEventKind::Content {
            delta: "late",
            reasoning_delta: None,
            block: None,
        });
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(*recorded.lock().unwrap(), vec![(9, 1), (9, 2), (9, 3)]);
    }
    #[test]
    fn slow_observer_applies_backpressure_without_a_background_queue() {
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let release_rx = Mutex::new(release_rx);
        let sink = |_: &AgentEvent<'_>| {
            entered_tx.send(()).unwrap();
            release_rx.lock().unwrap().recv().unwrap();
            EventDelivery::Accepted
        };
        let events = AgentEvents::new(identity(), &sink, None, 1);
        std::thread::scope(|scope| {
            let producer = scope.spawn(|| events.emit(AgentEventKind::RunStarted));
            entered_rx
                .recv_timeout(std::time::Duration::from_secs(1))
                .unwrap();
            assert!(!producer.is_finished());
            release_tx.send(()).unwrap();
            producer.join().unwrap();
        });
    }
}
