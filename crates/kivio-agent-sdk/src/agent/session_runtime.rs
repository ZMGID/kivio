use std::{
    collections::{BTreeMap, HashSet},
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, Weak,
    },
    task::{Context, Poll},
};

use futures::FutureExt;
use tokio::sync::{oneshot, Notify};

use super::{
    control::{AgentRunControl, AgentRunHandle, InputAdmission, StopAdmission},
    error::{AgentAdmissionError, AgentFailure, AgentRunError},
    events::{AgentEvent, AgentEventSink, AgentRunIdentity, EventDelivery, InputKind},
    history::RuntimeMessage,
    host::{AgentHost, AgentHostFuture},
    provider_runtime::ModelRuntime,
    session::{NoSessionPolicy, SessionPolicy},
    session_history::{complete_tool_rounds, EntryId, SessionEntry, SessionHistory},
    session_snapshot::{SessionSnapshot, SessionSnapshotConfig},
    types::{AgentRunConfig, AgentRunResult, AgentRunStatus, RunLimits},
    SteeringMessage,
};
use crate::{
    model::{GenerateOptions, MessagePart, ModelCapabilities, ModelRole},
    tools::ToolRegistry,
};

pub struct AgentSessionConfig {
    pub provider_runtime: Arc<dyn ModelRuntime>,
    pub provider_id: String,
    pub provider_label: String,
    pub api_format: String,
    pub model: String,
    pub options: GenerateOptions,
    pub capabilities: ModelCapabilities,
    pub conversation_id: String,
    pub history: SessionHistory,
    pub tools: ToolRegistry,
    pub host: Arc<dyn AgentHost>,
    pub limits: RunLimits,
    policy: tokio::sync::Mutex<Box<dyn SessionPolicy>>,
}

impl AgentSessionConfig {
    pub fn new(provider_runtime: Arc<dyn ModelRuntime>, model: impl Into<String>) -> Self {
        Self {
            provider_runtime,
            provider_id: String::new(),
            provider_label: String::new(),
            api_format: String::new(),
            model: model.into(),
            options: GenerateOptions::default(),
            capabilities: ModelCapabilities::default(),
            conversation_id: uuid::Uuid::new_v4().to_string(),
            history: SessionHistory::default(),
            tools: ToolRegistry::new(),
            host: Arc::new(MemoryHost),
            limits: RunLimits::default(),
            policy: tokio::sync::Mutex::new(Box::new(NoSessionPolicy)),
        }
    }

    pub fn with_policy(mut self, policy: impl SessionPolicy + 'static) -> Self {
        self.policy = tokio::sync::Mutex::new(Box::new(policy));
        self
    }

    /// Restores history and settings; the host must rebind tools, hooks and authority.
    pub fn from_snapshot(
        provider_runtime: Arc<dyn ModelRuntime>,
        snapshot: SessionSnapshot,
    ) -> Result<Self, String> {
        let saved = snapshot
            .config()
            .ok_or("History-only snapshot requires host-provided session configuration")?
            .clone();
        let mut config = Self::new(provider_runtime, saved.model);
        config.provider_id = saved.provider_id;
        config.provider_label = saved.provider_label;
        config.api_format = saved.api_format;
        config.conversation_id = saved.conversation_id;
        config.options = saved.options.into();
        config.capabilities = saved.capabilities;
        config.history = snapshot.into_history();
        Ok(config)
    }
}

struct MemoryHost;
impl AgentHost for MemoryHost {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionStatus {
    Idle,
    Running,
    StopRequested,
    Finishing,
}

#[derive(Debug, Clone)]
pub struct SessionPendingInput {
    pub generation: u64,
    pub kind: InputKind,
    pub message: SteeringMessage,
}

pub struct AgentSession {
    inner: Arc<SessionInner>,
}

struct SessionInner {
    config: AgentSessionConfig,
    state: Mutex<SessionState>,
}

struct SessionState {
    history: SessionHistory,
    tools: ToolRegistry,
    generation: u64,
    active: Option<Arc<Invocation>>,
    pending: Vec<SessionPendingInput>,
    subscriptions: BTreeMap<u64, Arc<dyn AgentEventSink>>,
    next_subscription: u64,
}

struct Invocation {
    generation: u64,
    base_leaf: Option<EntryId>,
    run_id: String,
    message_id: String,
    state: Mutex<InvocationState>,
    inactive: Notify,
    settled: AtomicBool,
    settlement: Notify,
}

struct InvocationState {
    status: SessionStatus,
    steer: Vec<SteeringMessage>,
    follow_up: Vec<SteeringMessage>,
    input_ids: HashSet<String>,
}

impl SessionState {
    fn owns_invocation(&self, invocation: &Arc<Invocation>) -> bool {
        self.generation == invocation.generation
            && self.active.as_ref().is_some_and(|active| {
                Arc::ptr_eq(active, invocation) && active.base_leaf == invocation.base_leaf
            })
    }
}

enum SessionStart {
    Prompt(RuntimeMessage),
    Continue,
    Regenerate {
        anchor: EntryId,
        replacement: Option<RuntimeMessage>,
    },
}

impl AgentSession {
    pub fn new(mut config: AgentSessionConfig) -> Self {
        let history = std::mem::take(&mut config.history);
        let tools = std::mem::take(&mut config.tools);
        Self {
            inner: Arc::new(SessionInner {
                config,
                state: Mutex::new(SessionState {
                    history,
                    tools,
                    generation: 0,
                    active: None,
                    pending: vec![],
                    subscriptions: BTreeMap::new(),
                    next_subscription: 0,
                }),
            }),
        }
    }

    /// Admission happens immediately. Dropping the returned waiter requests stop and retains cleanup.
    pub fn prompt(&self, input: impl Into<RuntimeMessage>) -> SessionRun {
        let input = input.into();
        if !valid_user_input(&input) {
            return SessionRun::rejected(AgentAdmissionError::EmptyInput);
        }
        self.start(SessionStart::Prompt(input))
    }

    pub fn continue_run(&self) -> SessionRun {
        self.start(SessionStart::Continue)
    }

    /// Reserves the run and selects its history together; rejected admission preserves the tree.
    pub fn regenerate(&self, anchor: EntryId, replacement: Option<RuntimeMessage>) -> SessionRun {
        if replacement
            .as_ref()
            .is_some_and(|input| !valid_user_input(input))
        {
            return SessionRun::rejected(AgentAdmissionError::EmptyInput);
        }
        self.start(SessionStart::Regenerate {
            anchor,
            replacement,
        })
    }

    fn start(&self, start: SessionStart) -> SessionRun {
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return SessionRun::rejected(AgentAdmissionError::NoRuntime);
        };
        let (invocation, history, tools) = {
            let mut state = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
            if state.active.is_some() {
                return SessionRun::rejected(AgentAdmissionError::Busy);
            }
            let mut history = state.history.clone();
            let input = match start {
                SessionStart::Prompt(input) => Some(input),
                SessionStart::Continue => None,
                SessionStart::Regenerate {
                    anchor,
                    replacement,
                } => {
                    if let Err(message) = history.prepare_regenerate(anchor, replacement) {
                        return SessionRun::rejected(AgentAdmissionError::InvalidHistory(message));
                    }
                    None
                }
            };
            if !complete_tool_rounds(history.context())
                || (input.is_none()
                    && !matches!(
                        history.context().last().and_then(RuntimeMessage::role),
                        Some(ModelRole::User | ModelRole::Tool)
                    ))
            {
                return SessionRun::rejected(AgentAdmissionError::InvalidContinuation);
            }
            if let Some(input) = input {
                history.push(input);
            }
            state.generation = state
                .generation
                .checked_add(1)
                .expect("session generation exhausted");
            let invocation = Arc::new(Invocation {
                generation: state.generation,
                base_leaf: history.active_leaf(),
                run_id: uuid::Uuid::new_v4().to_string(),
                message_id: uuid::Uuid::new_v4().to_string(),
                state: Mutex::new(InvocationState {
                    status: SessionStatus::Running,
                    steer: vec![],
                    follow_up: vec![],
                    input_ids: HashSet::new(),
                }),
                inactive: Notify::new(),
                settled: AtomicBool::new(false),
                settlement: Notify::new(),
            });
            state.history = history.clone();
            state.active = Some(invocation.clone());
            (invocation, history, state.tools.snapshot())
        };
        let (sender, receiver) = oneshot::channel();
        let inner = self.inner.clone();
        let worker = invocation.clone();
        runtime.spawn(async move {
            run_session(inner, worker, history, tools, sender).await;
        });
        SessionRun {
            receiver: Some(receiver),
            immediate: None,
            invocation: Some(invocation),
            inner: Arc::downgrade(&self.inner),
        }
    }

    pub fn status(&self) -> SessionStatus {
        self.active()
            .map(|active| active.status())
            .unwrap_or(SessionStatus::Idle)
    }

    pub fn context(&self) -> Vec<RuntimeMessage> {
        self.inner
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .history
            .context()
            .to_vec()
    }

    pub fn entries(&self) -> Vec<SessionEntry> {
        self.inner
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .history
            .entries()
            .to_vec()
    }

    /// Exports settled history and standard settings without live runtime authority.
    pub fn snapshot(&self) -> Result<SessionSnapshot, AgentAdmissionError> {
        self.navigate(|history| {
            let config = &self.inner.config;
            Ok(history.snapshot()?.with_config(SessionSnapshotConfig {
                provider_id: config.provider_id.clone(),
                provider_label: config.provider_label.clone(),
                api_format: config.api_format.clone(),
                model: config.model.clone(),
                conversation_id: config.conversation_id.clone(),
                options: (&config.options).into(),
                capabilities: config.capabilities.clone(),
            }))
        })
    }

    pub fn active_leaf(&self) -> Option<EntryId> {
        self.inner
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .history
            .active_leaf()
    }

    pub fn leaves(&self) -> Vec<EntryId> {
        self.inner
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .history
            .leaves()
    }

    pub fn switch_branch(&self, leaf: Option<EntryId>) -> Result<(), AgentAdmissionError> {
        self.navigate(|history| history.switch_branch(leaf))
    }

    pub fn fork(&self, anchor: EntryId) -> Result<(), AgentAdmissionError> {
        self.navigate(|history| history.fork(anchor))
    }

    pub fn rewind(&self, user: EntryId) -> Result<RuntimeMessage, AgentAdmissionError> {
        self.navigate(|history| history.rewind(user))
    }

    fn navigate<T>(
        &self,
        navigate: impl FnOnce(&mut SessionHistory) -> Result<T, String>,
    ) -> Result<T, AgentAdmissionError> {
        let mut state = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.active.is_some() {
            return Err(AgentAdmissionError::Busy);
        }
        navigate(&mut state.history).map_err(AgentAdmissionError::InvalidHistory)
    }

    pub fn steer(&self, message: SteeringMessage) -> InputAdmission {
        self.active()
            .map(|active| active.submit(InputKind::Steer, message))
            .unwrap_or(InputAdmission::Inactive)
    }

    pub fn follow_up(&self, message: SteeringMessage) -> InputAdmission {
        self.active()
            .map(|active| active.submit(InputKind::FollowUp, message))
            .unwrap_or(InputAdmission::Inactive)
    }

    pub fn abort(&self) -> StopAdmission {
        self.active()
            .map(|active| active.stop())
            .unwrap_or(StopAdmission::Inactive)
    }

    /// Captures the current generation now; a later run cannot extend this wait.
    pub fn wait_inactive(&self) -> impl Future<Output = ()> + Send + 'static {
        let active = self.active();
        async move {
            if let Some(active) = active {
                let notified = active.settlement.notified();
                tokio::pin!(notified);
                notified.as_mut().enable();
                if !active.settled.load(Ordering::Acquire) {
                    notified.await;
                }
            }
        }
    }

    pub fn unconsumed_inputs(&self) -> Vec<SessionPendingInput> {
        self.inner
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .pending
            .clone()
    }

    pub fn take_unconsumed_inputs(&self) -> Vec<SessionPendingInput> {
        std::mem::take(
            &mut self
                .inner
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .pending,
        )
    }

    pub fn set_policy(
        &self,
        policy: impl SessionPolicy + 'static,
    ) -> Result<(), AgentAdmissionError> {
        let state = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.active.is_some() {
            return Err(AgentAdmissionError::Busy);
        }
        let mut current = self
            .inner
            .config
            .policy
            .try_lock()
            .map_err(|_| AgentAdmissionError::Busy)?;
        *current = Box::new(policy);
        Ok(())
    }

    pub fn set_tools(&self, tools: ToolRegistry) -> Result<(), AgentAdmissionError> {
        let mut state = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.active.is_some() {
            return Err(AgentAdmissionError::Busy);
        }
        state.tools = tools;
        Ok(())
    }

    pub fn subscribe(&self, sink: Arc<dyn AgentEventSink>) -> SessionSubscription {
        let mut state = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
        let id = state.next_subscription;
        state.next_subscription = id.checked_add(1).expect("subscription ID exhausted");
        state.subscriptions.insert(id, sink);
        SessionSubscription {
            inner: Arc::downgrade(&self.inner),
            id,
        }
    }

    fn active(&self) -> Option<Arc<Invocation>> {
        self.inner
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .active
            .clone()
    }
}

fn valid_user_input(input: &RuntimeMessage) -> bool {
    input.model_message().is_some_and(|message| {
        message.role == ModelRole::User
            && message.content.iter().any(|part| match part {
                MessagePart::Text { text } => !text.trim().is_empty(),
                _ => true,
            })
    })
}

impl Drop for AgentSession {
    fn drop(&mut self) {
        self.abort();
    }
}

impl Invocation {
    fn status(&self) -> SessionStatus {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).status
    }
    fn finishing(&self) {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).status = SessionStatus::Finishing;
        self.inactive.notify_waiters();
    }
}

impl AgentRunControl for Invocation {
    fn active(&self) -> bool {
        self.status() == SessionStatus::Running
    }
    fn stop(&self) -> StopAdmission {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.status != SessionStatus::Running {
            return StopAdmission::Inactive;
        }
        state.status = SessionStatus::StopRequested;
        self.inactive.notify_waiters();
        StopAdmission::Requested
    }
    fn submit(&self, kind: InputKind, message: SteeringMessage) -> InputAdmission {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.status != SessionStatus::Running {
            return InputAdmission::Inactive;
        }
        if !state.input_ids.insert(message.id.clone()) {
            return InputAdmission::Duplicate;
        }
        match kind {
            InputKind::Steer => state.steer.push(message),
            InputKind::FollowUp => state.follow_up.push(message),
        }
        InputAdmission::Accepted
    }
    fn take(&self, kind: InputKind) -> Vec<SteeringMessage> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.status != SessionStatus::Running {
            return vec![];
        }
        std::mem::take(match kind {
            InputKind::Steer => &mut state.steer,
            InputKind::FollowUp => &mut state.follow_up,
        })
    }
    fn wait_inactive(&self) -> AgentHostFuture<'_, ()> {
        Box::pin(async move {
            let notified = self.inactive.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.active() {
                notified.await;
            }
        })
    }
}

impl AgentEventSink for SessionInner {
    fn on_event(&self, event: &AgentEvent<'_>) -> EventDelivery {
        let subscriptions: Vec<_> = self
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .subscriptions
            .iter()
            .map(|(id, sink)| (*id, sink.clone()))
            .collect();
        for (id, sink) in subscriptions {
            if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| sink.on_event(event)))
                .unwrap_or(EventDelivery::Closed)
                == EventDelivery::Closed
            {
                self.state
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .subscriptions
                    .remove(&id);
            }
        }
        EventDelivery::Accepted
    }
}

pub struct SessionSubscription {
    inner: Weak<SessionInner>,
    id: u64,
}
impl Drop for SessionSubscription {
    fn drop(&mut self) {
        if let Some(inner) = self.inner.upgrade() {
            inner
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .subscriptions
                .remove(&self.id);
        }
    }
}

pub struct SessionRun {
    receiver: Option<oneshot::Receiver<Result<AgentRunResult, AgentRunError>>>,
    immediate: Option<AgentRunError>,
    invocation: Option<Arc<Invocation>>,
    inner: Weak<SessionInner>,
}

impl SessionRun {
    fn rejected(error: AgentAdmissionError) -> Self {
        Self {
            receiver: None,
            immediate: Some(AgentRunError::Admission(error)),
            invocation: None,
            inner: Weak::new(),
        }
    }
}

impl Future for SessionRun {
    type Output = Result<AgentRunResult, AgentRunError>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        if let Some(error) = this.immediate.take() {
            return Poll::Ready(Err(error));
        }
        match Pin::new(
            this.receiver
                .as_mut()
                .expect("completed SessionRun polled again"),
        )
        .poll(cx)
        {
            Poll::Pending => Poll::Pending,
            Poll::Ready(result) => {
                this.receiver = None;
                this.invocation = None;
                Poll::Ready(result.unwrap_or_else(|_| {
                    let history = this
                        .inner
                        .upgrade()
                        .map(|inner| {
                            inner
                                .state
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .history
                                .clone()
                        })
                        .unwrap_or_default();
                    Err(worker_failure(history))
                }))
            }
        }
    }
}

impl Drop for SessionRun {
    fn drop(&mut self) {
        if let Some(invocation) = &self.invocation {
            invocation.stop();
        }
    }
}

async fn run_session(
    inner: Arc<SessionInner>,
    invocation: Arc<Invocation>,
    history: SessionHistory,
    tools: ToolRegistry,
    sender: oneshot::Sender<Result<AgentRunResult, AgentRunError>>,
) {
    let cfg = &inner.config;
    let identity = AgentRunIdentity {
        conversation_id: &cfg.conversation_id,
        run_id: &invocation.run_id,
        message_id: &invocation.message_id,
        generation: invocation.generation,
    };
    let config = AgentRunConfig {
        provider_runtime: cfg.provider_runtime.as_ref(),
        conversation_id: cfg.conversation_id.clone(),
        run_id: invocation.run_id.clone(),
        message_id: invocation.message_id.clone(),
        generation: invocation.generation,
        provider_id: cfg.provider_id.clone(),
        provider_label: cfg.provider_label.clone(),
        api_format: cfg.api_format.clone(),
        model: cfg.model.clone(),
        history: history.clone(),
        tools: tools.definitions(),
        blocked_tool_calls: vec![],
        options: cfg.options.clone(),
        capabilities: cfg.capabilities.clone(),
        initial_cache_usage: None,
        report_context: false,
        limits: cfg.limits.clone(),
    };
    let mut policy = cfg.policy.lock().await;
    let mut result = match std::panic::AssertUnwindSafe(super::run_agent_loop(
        config,
        cfg.host.as_ref(),
        &tools,
        policy.as_mut(),
        inner.as_ref(),
        AgentRunHandle::new(identity, invocation.as_ref()),
    ))
    .catch_unwind()
    .await
    {
        Ok(result) => result,
        Err(_) => Err(worker_failure(history)),
    };
    invocation.finishing();
    let partial = match &result {
        Ok(result) => Some(result),
        Err(AgentRunError::Failed { partial, .. }) => Some(partial.as_ref()),
        Err(AgentRunError::Admission(_)) => None,
    };
    if let Some(partial) = partial {
        let installed = {
            let mut state = inner.state.lock().unwrap_or_else(|e| e.into_inner());
            if state.owns_invocation(&invocation)
                && state.history.active_leaf() == invocation.base_leaf
            {
                state.history = partial.history.clone();
                true
            } else {
                false
            }
        };
        let (operation, commit) = if installed {
            (
                "session.commit_run",
                std::panic::AssertUnwindSafe(async {
                    cfg.host.commit_run(identity, partial).await
                })
                .catch_unwind()
                .await
                .unwrap_or_else(|_| Err("Session commit panicked".into())),
            )
        } else {
            (
                "session.history",
                Err("Session invocation no longer owns its history branch".into()),
            )
        };
        if let Err(message) = commit {
            let mut partial = match result {
                Ok(result) => Box::new(result),
                Err(AgentRunError::Failed { partial, .. }) => partial,
                Err(AgentRunError::Admission(_)) => unreachable!(),
            };
            let cause = AgentFailure::Host { operation, message };
            partial.status = AgentRunStatus::Failed(cause.clone());
            result = Err(AgentRunError::Failed { cause, partial });
        }
    }
    drop(policy);
    let mut pending = Vec::new();
    let core_result = match &mut result {
        Ok(result) => Some(result),
        Err(AgentRunError::Failed { partial, .. }) => Some(partial.as_mut()),
        _ => None,
    };
    if let Some(core_result) = core_result {
        pending.extend(core_result.pending_steering.iter().cloned().map(|message| {
            SessionPendingInput {
                generation: invocation.generation,
                kind: InputKind::Steer,
                message,
            }
        }));
        pending.extend(
            core_result
                .pending_follow_up
                .iter()
                .cloned()
                .map(|message| SessionPendingInput {
                    generation: invocation.generation,
                    kind: InputKind::FollowUp,
                    message,
                }),
        );
    }
    {
        let mut control = invocation.state.lock().unwrap_or_else(|e| e.into_inner());
        pending.extend(
            std::mem::take(&mut control.steer)
                .into_iter()
                .map(|message| SessionPendingInput {
                    generation: invocation.generation,
                    kind: InputKind::Steer,
                    message,
                }),
        );
        pending.extend(
            std::mem::take(&mut control.follow_up)
                .into_iter()
                .map(|message| SessionPendingInput {
                    generation: invocation.generation,
                    kind: InputKind::FollowUp,
                    message,
                }),
        );
    }
    {
        let mut state = inner.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.owns_invocation(&invocation) {
            state.pending.extend(pending);
            state.active = None;
        }
        invocation.settled.store(true, Ordering::Release);
        invocation.settlement.notify_waiters();
    }
    let _ = sender.send(result);
}

fn worker_failure(history: SessionHistory) -> AgentRunError {
    let cause = AgentFailure::Host {
        operation: "session.worker",
        message: "Session worker stopped before returning a result".into(),
    };
    AgentRunError::Failed {
        cause: cause.clone(),
        partial: Box::new(AgentRunResult {
            final_message: None,
            content: String::new(),
            reasoning: None,
            tool_records: vec![],
            segments: vec![],
            generated_history: vec![],
            status: AgentRunStatus::Failed(cause),
            usage: None,
            cache_pairs: None,
            last_step_usage: None,
            history,
            compacted: false,
            images: vec![],
            pending_steering: vec![],
            pending_follow_up: vec![],
        }),
    }
}
