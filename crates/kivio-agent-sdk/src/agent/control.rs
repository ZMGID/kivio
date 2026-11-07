use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, Mutex,
};

use super::events::{AgentRunIdentity, InputKind};
use super::{AgentHostFuture, SteeringMessage};

static NEXT_INVOCATION_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputAdmission {
    Accepted,
    Duplicate,
    Inactive,
    /// This host receives input through a separate application operation, such as IRC.
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopAdmission {
    Requested,
    Inactive,
}

/// A capability already bound to an admitted run and its application owner.
pub trait AgentRunControl: Send + Sync {
    fn active(&self) -> bool;
    fn stop(&self) -> StopAdmission;
    fn submit(&self, kind: InputKind, message: SteeringMessage) -> InputAdmission;
    fn take(&self, kind: InputKind) -> Vec<SteeringMessage>;
    fn wait_inactive(&self) -> AgentHostFuture<'_, ()>;
}

#[derive(Default)]
struct CoreCompletion {
    phase: Mutex<CorePhase>,
    changed: tokio::sync::Notify,
}

#[derive(Default, PartialEq, Eq)]
enum CorePhase {
    #[default]
    Ready,
    Running,
    Finished,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunStartError {
    AlreadyRunning,
    Finished,
}

impl std::fmt::Display for RunStartError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::AlreadyRunning => "Agent run is already running",
            Self::Finished => "Agent run is already finished",
        })
    }
}

/// Inputs share the host's existing mailbox. Acceptance means queued, not consumed.
/// Inactivity ends core admission; outer persistence and cleanup may still hold capacity.
#[derive(Clone)]
pub struct AgentRunHandle<'a> {
    identity: AgentRunIdentity<'a>,
    invocation_id: u64,
    control: &'a dyn AgentRunControl,
    completion: Arc<CoreCompletion>,
}

impl<'a> AgentRunHandle<'a> {
    pub fn new(identity: AgentRunIdentity<'a>, control: &'a dyn AgentRunControl) -> Self {
        Self {
            identity,
            invocation_id: NEXT_INVOCATION_ID
                .try_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
                .expect("agent invocation ID space exhausted"),
            control,
            completion: Arc::default(),
        }
    }

    pub fn identity(&self) -> AgentRunIdentity<'a> {
        self.identity
    }

    /// Identifies this core invocation within the process; clones share the ID.
    pub fn invocation_id(&self) -> u64 {
        self.invocation_id
    }

    pub fn active(&self) -> bool {
        *self.phase() != CorePhase::Finished && self.control.active()
    }

    /// Requests cooperative stop. It does not release the application's run slot.
    pub fn stop(&self) -> StopAdmission {
        let phase = self.phase();
        if *phase == CorePhase::Finished {
            StopAdmission::Inactive
        } else {
            self.control.stop()
        }
    }

    pub fn steer(&self, message: SteeringMessage) -> InputAdmission {
        self.submit(InputKind::Steer, message)
    }

    pub fn follow_up(&self, message: SteeringMessage) -> InputAdmission {
        self.submit(InputKind::FollowUp, message)
    }

    pub fn take_steering(&self) -> Vec<SteeringMessage> {
        self.take(InputKind::Steer)
    }

    pub fn take_follow_up(&self) -> Vec<SteeringMessage> {
        self.take(InputKind::FollowUp)
    }

    pub async fn wait_inactive(&self) {
        let notified = self.completion.changed.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        if !self.active() {
            return;
        }
        tokio::select! {
            _ = notified => {},
            _ = self.control.wait_inactive() => {},
        }
    }

    pub(crate) fn finish_core(&self) {
        *self.phase() = CorePhase::Finished;
        self.completion.changed.notify_waiters();
    }

    pub(crate) fn begin_core(&self) -> Result<(), RunStartError> {
        let mut phase = self.phase();
        match *phase {
            CorePhase::Ready => {
                *phase = CorePhase::Running;
                Ok(())
            }
            CorePhase::Running => Err(RunStartError::AlreadyRunning),
            CorePhase::Finished => Err(RunStartError::Finished),
        }
    }

    fn submit(&self, kind: InputKind, message: SteeringMessage) -> InputAdmission {
        let phase = self.phase();
        if *phase == CorePhase::Finished {
            InputAdmission::Inactive
        } else {
            self.control.submit(kind, message)
        }
    }

    fn take(&self, kind: InputKind) -> Vec<SteeringMessage> {
        let phase = self.phase();
        if *phase == CorePhase::Finished {
            Vec::new()
        } else {
            self.control.take(kind)
        }
    }

    fn phase(&self) -> std::sync::MutexGuard<'_, CorePhase> {
        self.completion
            .phase
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;
    #[derive(Default)]
    struct TestControl {
        stopped: AtomicBool,
        steering: Mutex<Vec<SteeringMessage>>,
        follow_up: Mutex<Vec<SteeringMessage>>,
        changed: tokio::sync::Notify,
    }
    impl AgentRunControl for TestControl {
        fn active(&self) -> bool {
            !self.stopped.load(Ordering::SeqCst)
        }
        fn stop(&self) -> StopAdmission {
            if self.stopped.swap(true, Ordering::SeqCst) {
                return StopAdmission::Inactive;
            }
            self.changed.notify_waiters();
            StopAdmission::Requested
        }
        fn submit(&self, kind: InputKind, message: SteeringMessage) -> InputAdmission {
            if !self.active() {
                return InputAdmission::Inactive;
            }
            match kind {
                InputKind::Steer => self.steering.lock().unwrap().push(message),
                InputKind::FollowUp => self.follow_up.lock().unwrap().push(message),
            }
            InputAdmission::Accepted
        }
        fn take(&self, kind: InputKind) -> Vec<SteeringMessage> {
            if !self.active() {
                return Vec::new();
            }
            std::mem::take(&mut *match kind {
                InputKind::Steer => self.steering.lock().unwrap(),
                InputKind::FollowUp => self.follow_up.lock().unwrap(),
            })
        }
        fn wait_inactive(&self) -> AgentHostFuture<'_, ()> {
            Box::pin(async {
                let changed = self.changed.notified();
                tokio::pin!(changed);
                changed.as_mut().enable();
                if self.active() {
                    changed.await;
                }
            })
        }
    }

    fn message(id: &str) -> SteeringMessage {
        SteeringMessage::new(id.into(), id).unwrap()
    }

    fn identity(generation: u64) -> AgentRunIdentity<'static> {
        AgentRunIdentity {
            conversation_id: "conversation",
            run_id: "run",
            message_id: "message",
            generation,
        }
    }

    #[test]
    fn handles_for_the_same_run_have_distinct_invocations_and_clones_share_one() {
        let generation = 1;
        let control = TestControl::default();
        let first = AgentRunHandle::new(identity(generation), &control);
        let clone = first.clone();
        let second = AgentRunHandle::new(identity(generation), &control);

        assert!(first.invocation_id() < second.invocation_id());
        assert_eq!(first.invocation_id(), clone.invocation_id());
        assert_eq!(first.begin_core(), Ok(()));
        assert_eq!(clone.begin_core(), Err(RunStartError::AlreadyRunning));
        assert_eq!(second.begin_core(), Ok(()));
    }

    #[tokio::test]
    async fn core_completion_closes_clones_and_wakes_waiters_while_host_stays_active() {
        let generation = 1;
        let control = TestControl::default();
        let handle = AgentRunHandle::new(identity(generation), &control);
        let clone = handle.clone();
        let waiting = clone.wait_inactive();
        tokio::pin!(waiting);
        assert!(futures::poll!(waiting.as_mut()).is_pending());
        assert_eq!(handle.steer(message("accepted")), InputAdmission::Accepted);
        assert_eq!(handle.begin_core(), Ok(()));
        assert_eq!(clone.begin_core(), Err(RunStartError::AlreadyRunning));

        handle.finish_core();

        assert!(futures::poll!(waiting.as_mut()).is_ready());
        assert!(!clone.active());
        assert_eq!(clone.begin_core(), Err(RunStartError::Finished));
        assert_eq!(clone.steer(message("late")), InputAdmission::Inactive);
        assert_eq!(clone.follow_up(message("late")), InputAdmission::Inactive);
        assert_eq!(clone.stop(), StopAdmission::Inactive);
        assert!(clone.take_steering().is_empty());
        assert!(control.active());
        assert_eq!(control.take(InputKind::Steer), vec![message("accepted")]);
    }

    #[tokio::test]
    async fn stopped_handle_cannot_reach_another_control_or_its_input() {
        let old_generation = 1;
        let old_control = TestControl::default();
        let old = AgentRunHandle::new(identity(old_generation), &old_control);
        assert_eq!(old.steer(message("old")), InputAdmission::Accepted);
        assert_eq!(old.follow_up(message("old")), InputAdmission::Accepted);
        let waiting = old.wait_inactive();
        tokio::pin!(waiting);
        assert!(futures::poll!(waiting.as_mut()).is_pending());
        assert_eq!(old.stop(), StopAdmission::Requested);
        assert!(futures::poll!(waiting.as_mut()).is_ready());

        let generation = 2;
        let control = TestControl::default();
        let current = AgentRunHandle::new(identity(generation), &control);
        assert!(current.take_steering().is_empty());
        assert!(current.take_follow_up().is_empty());
        assert_eq!(current.steer(message("new")), InputAdmission::Accepted);
        assert_eq!(current.follow_up(message("new")), InputAdmission::Accepted);
        assert_eq!(old.stop(), StopAdmission::Inactive);
        assert_eq!(old.steer(message("late")), InputAdmission::Inactive);
        assert_eq!(old.follow_up(message("late")), InputAdmission::Inactive);
        assert!(old.take_steering().is_empty());
        assert!(old.take_follow_up().is_empty());
        assert_eq!(current.take_steering(), vec![message("new")]);
        assert_eq!(current.take_follow_up(), vec![message("new")]);
        assert!(current.active());
    }
}
