use crate::chat::agent::history::RuntimeMessage;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::watch;

use super::{
    executor, lifecycle,
    registry::{AgentRecord, AgentStatus},
    session::{Control, Slot},
    MAIN_AGENT_ID,
};
use crate::state::AppState;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    pub id: String,
    pub root: String,
    pub from: String,
    pub to: String,
    pub body: String,
    pub sequence: u64,
    pub reply_to: Option<String>,
    pub progress: bool,
    pub broadcast: bool,
    pub wake_relay: bool,
    pub timestamp: i64,
}

impl Message {
    fn asks_reply(&self) -> bool {
        !self.progress && !self.broadcast && !self.wake_relay && self.from != "@user"
    }

    pub fn model_input(&self) -> Value {
        let content = super::prompts::render(
            include_str!("prompts/irc-incoming.md"),
            &json!({
                "from": super::delivery::escape_harness_tags(&self.from), "replyTo": self.reply_to,
                "progress": self.progress, "message": super::delivery::escape_harness_tags(&self.body),
                "interrupting": false, "relayOnStop": self.asks_reply(),
            }),
        );
        let mut input = super::delivery::report_input(&content);
        input["_ui_message_id"] = json!(format!("subagent-irc-{}", self.id));
        input[super::delivery::REPORT_MESSAGE_ID_KEY] = json!(format!("subagent-irc-{}", self.id));
        input["_subagent_irc_id"] = json!(self.id);
        input
    }

    pub(crate) fn worker_input(&self, parent: &str) -> Value {
        if self.broadcast || (self.from != parent && self.from != "@user") {
            return self.model_input();
        }
        let source = if self.from == "@user" {
            "the user".to_string()
        } else {
            format!("your parent agent `{}`", self.from)
        };
        json!({"role":"user", "content":format!("Message from {source}:\n{}", self.body),
            "_subagent_irc_id":self.id, super::delivery::REPORT_MESSAGE_ID_KEY:format!("subagent-irc-{}", self.id)})
    }

    fn view(&self, observation: bool) -> super::view::SubagentIrcMessage {
        super::view::SubagentIrcMessage {
            id: self.id.clone(),
            conversation_id: self.root.clone(),
            from_agent: self.from.clone(),
            to_agent: self.to.clone(),
            content: self.body.clone(),
            reply_to: self.reply_to.clone(),
            progress: self.progress,
            broadcast: self.broadcast,
            wake_relay: Some(self.wake_relay),
            observation,
            timestamp: self.timestamp,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SendReceipt {
    pub id: String,
    pub recipient: String,
    pub status: String,
    pub receipts: Vec<Receipt>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Receipt {
    pub id: String,
    pub recipient: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Clone)]
pub(crate) struct Debt {
    pub target: String,
    pub sequence: u64,
    pub promised: bool,
}

#[derive(Clone)]
struct QueuedMessage {
    message: Message,
    answers_debt: bool,
}

#[derive(Default)]
struct Inner {
    sequence: u64,
    incoming: HashMap<(String, String), VecDeque<QueuedMessage>>,
    failed: HashMap<(String, String), VecDeque<QueuedMessage>>,
    debts: HashMap<(String, String), Vec<Debt>>,
    waking: HashMap<(String, String, u64), Vec<Message>>,
    sent: HashMap<(String, String, String), u64>,
}

pub struct Bus {
    inner: Mutex<Inner>,
    changed: watch::Sender<u64>,
}

impl Default for Bus {
    fn default() -> Self {
        Self {
            inner: Mutex::new(Inner::default()),
            changed: watch::channel(0).0,
        }
    }
}

pub(crate) struct IncomingClaim<'a> {
    bus: &'a Bus,
    messages: Vec<Message>,
    queued: Vec<QueuedMessage>,
    committed: bool,
}

impl IncomingClaim<'_> {
    pub fn messages(&self) -> &[Message] {
        &self.messages
    }
    pub fn commit(&mut self) {
        self.committed = true;
    }
}

impl Drop for IncomingClaim<'_> {
    fn drop(&mut self) {
        if self.committed || self.messages.is_empty() {
            return;
        }
        let mut inner = self
            .bus
            .inner
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        for queued in self.queued.iter().rev() {
            let message = &queued.message;
            inner
                .incoming
                .entry((message.root.clone(), message.to.clone()))
                .or_default()
                .push_front(queued.clone());
        }
        drop(inner);
        self.bus.bump();
    }
}

impl Bus {
    fn bump(&self) {
        self.changed
            .send_modify(|sequence| *sequence = sequence.wrapping_add(1));
    }
    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.changed.subscribe()
    }
    pub fn begin_turn(&self, root: &str, owner: &str) {
        self.inner
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .debts
            .remove(&(root.into(), owner.into()));
    }
    pub fn has_pending(&self, root: &str, id: &str) -> bool {
        let inner = self.inner.lock().unwrap_or_else(|error| error.into_inner());
        inner
            .incoming
            .get(&(root.into(), id.into()))
            .is_some_and(|messages| !messages.is_empty())
            || inner
                .failed
                .get(&(root.into(), id.into()))
                .is_some_and(|messages| !messages.is_empty())
    }

    pub fn has_pending_main(&self, root: &str) -> bool {
        self.has_pending(root, MAIN_AGENT_ID)
    }

    pub(crate) fn mark(&self) -> u64 {
        self.inner
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .sequence
    }

    fn enqueue(&self, message: Message) {
        let mut inner = self.inner.lock().unwrap_or_else(|error| error.into_inner());
        let debt_key = (message.root.clone(), message.to.clone());
        let mut answers_debt = false;
        if let Some(debts) = inner.debts.get_mut(&debt_key) {
            debts.retain_mut(|debt| {
                if debt.target != message.from || message.sequence <= debt.sequence {
                    return true;
                }
                if message.progress {
                    debt.promised = true;
                    true
                } else {
                    answers_debt = true;
                    false
                }
            });
        }
        inner
            .incoming
            .entry((message.root.clone(), message.to.clone()))
            .or_default()
            .push_back(QueuedMessage {
                message,
                answers_debt,
            });
        drop(inner);
        self.bump();
    }

    #[cfg(test)]
    pub(super) fn seed_message(&self, message: Message) {
        {
            let mut inner = self.inner.lock().unwrap();
            inner.sequence = inner.sequence.max(message.sequence);
            if message.asks_reply() {
                let debts = inner
                    .debts
                    .entry((message.root.clone(), message.from.clone()))
                    .or_default();
                debts.retain(|debt| debt.target != message.to);
                debts.push(Debt {
                    target: message.to.clone(),
                    sequence: message.sequence,
                    promised: false,
                });
            }
        }
        self.enqueue(message);
    }

    pub(crate) fn claim<'a>(
        &'a self,
        root: &str,
        id: &str,
        eligible: impl Fn(&Message) -> bool,
    ) -> IncomingClaim<'a> {
        self.claim_matching(root, id, |queued| eligible(&queued.message))
    }

    pub(crate) fn claim_wait<'a>(
        &'a self,
        root: &str,
        id: &str,
        parent: Option<&str>,
    ) -> IncomingClaim<'a> {
        self.claim_matching(root, id, |queued| {
            let message = &queued.message;
            (!message.progress || parent == Some(message.from.as_str()))
                && (!message.broadcast || queued.answers_debt)
        })
    }

    fn claim_matching<'a>(
        &'a self,
        root: &str,
        id: &str,
        eligible: impl Fn(&QueuedMessage) -> bool,
    ) -> IncomingClaim<'a> {
        let mut inner = self.inner.lock().unwrap_or_else(|error| error.into_inner());
        let key = (root.into(), id.into());
        let mut messages = Vec::new();
        let Inner {
            failed, incoming, ..
        } = &mut *inner;
        for queues in [failed, incoming] {
            if let Some(queue) = queues.get_mut(&key) {
                let mut keep = VecDeque::new();
                while let Some(message) = queue.pop_front() {
                    if eligible(&message) {
                        messages.push(message);
                    } else {
                        keep.push_back(message);
                    }
                }
                *queue = keep;
            }
        }
        messages.sort_by_key(|queued| queued.message.sequence);
        IncomingClaim {
            bus: self,
            messages: messages
                .iter()
                .map(|queued| queued.message.clone())
                .collect(),
            queued: messages,
            committed: false,
        }
    }

    pub(crate) fn pending_replies(&self, root: &str, owner: &str) -> Vec<Debt> {
        self.inner
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .debts
            .get(&(root.into(), owner.into()))
            .cloned()
            .unwrap_or_default()
    }

    pub(crate) fn release_debt(&self, root: &str, owner: &str, target: &str) {
        if let Some(debts) = self
            .inner
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .debts
            .get_mut(&(root.into(), owner.into()))
        {
            debts.retain(|debt| debt.target != target);
        }
        self.bump();
    }

    pub(crate) fn agent_stopped(&self, root: &str, _id: &str) {
        let _ = root;
        self.bump();
    }

    fn failed_handoff(&self, root: &str, id: &str) -> Vec<Message> {
        let mut inner = self.inner.lock().unwrap_or_else(|error| error.into_inner());
        let key = (root.into(), id.into());
        let messages = inner.incoming.remove(&key).unwrap_or_default();
        let failed = inner.failed.entry(key).or_default();
        failed.extend(messages.iter().cloned());
        while failed.len() > 100 {
            failed.pop_front();
        }
        messages.into_iter().map(|queued| queued.message).collect()
    }

    pub fn forget(&self, root: &str) {
        let mut inner = self.inner.lock().unwrap_or_else(|error| error.into_inner());
        inner.incoming.retain(|(owner, _), _| owner != root);
        inner.failed.retain(|(owner, _), _| owner != root);
        inner.debts.retain(|(owner, _), _| owner != root);
        inner.waking.retain(|(owner, _, _), _| owner != root);
        inner.sent.retain(|(owner, _, _), _| owner != root);
    }
}

fn parse(url: &str) -> Result<(&str, bool), String> {
    let rest = url
        .strip_prefix("agent://")
        .ok_or("Expected agent://<id>")?;
    let (id, query) = rest.split_once('?').unwrap_or((rest, ""));
    if id.is_empty() || id.contains('/') || !super::task::valid_agent_id(id) {
        return Err("Agent URL must name one recipient without a path".into());
    }
    let progress = query
        .split('&')
        .any(|part| part.split('=').next() == Some("progress"));
    if id == "all" && progress {
        return Err("Broadcast progress is not supported".into());
    }
    Ok((id, progress))
}

pub async fn send(
    app: &AppHandle,
    root: &str,
    sender_id: &str,
    url: &str,
    body: &str,
) -> Result<SendReceipt, String> {
    send_message(app, root, sender_id, url, body, None, false).await
}

async fn send_message(
    app: &AppHandle,
    root: &str,
    sender: &str,
    url: &str,
    body: &str,
    reply_to: Option<String>,
    wake_relay: bool,
) -> Result<SendReceipt, String> {
    crate::chat::storage::validate_conversation_id(root)?;
    if body.trim().is_empty() {
        return Err("Agent messages cannot be empty".into());
    }
    let (target, progress) = parse(url)?;
    let state = app.state::<AppState>();
    let manager = &state.sub_agents;
    if !wake_relay
        && sender != MAIN_AGENT_ID
        && sender != "@user"
        && !manager
            .registry
            .get(root, sender)
            .is_some_and(|record| record.status == AgentStatus::Running)
    {
        return Err("Sender is not an active agent".into());
    }
    if target.eq_ignore_ascii_case(sender) {
        return Err("Agents cannot send messages to themselves".into());
    }
    let broadcast = target == "all";
    if broadcast && sender != MAIN_AGENT_ID {
        return Err("Only Main can broadcast".into());
    }
    let sender_run = manager
        .registry
        .get(root, sender)
        .map(|record| (record.generation, record.run_id));
    let sequence = {
        let mut inner = manager
            .irc
            .inner
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        inner.sequence = inner.sequence.wrapping_add(1);
        inner.sequence
    };
    if target != MAIN_AGENT_ID && !manager.registry.contains(root, target) {
        lifecycle::discover(app, root).await?;
    }
    let targets = if broadcast {
        manager
            .registry
            .list_visible_to(root, sender)
            .into_iter()
            .filter(accepts_irc)
            .map(|record| record.id)
            .collect::<Vec<_>>()
    } else {
        vec![target.into()]
    };
    deliver_targets(target, targets, |recipient| {
        send_to_recipient(
            app,
            Message {
                id: uuid::Uuid::new_v4().to_string(),
                root: root.into(),
                from: sender.into(),
                to: recipient,
                body: body.into(),
                sequence,
                reply_to: reply_to.clone(),
                progress,
                broadcast,
                wake_relay,
                timestamp: super::registry::now_ms(),
            },
            sender_run.as_ref(),
        )
    })
    .await
}

fn accepts_irc(record: &AgentRecord) -> bool {
    record.agent_type != "advisor"
}

async fn deliver_targets<F, Fut>(
    target: &str,
    targets: Vec<String>,
    mut deliver: F,
) -> Result<SendReceipt, String>
where
    F: FnMut(String) -> Fut,
    Fut: std::future::Future<Output = Result<Receipt, String>>,
{
    let mut receipts = Vec::with_capacity(targets.len());
    for recipient in targets {
        let result = deliver(recipient.clone()).await;
        receipts.push(match result {
            Ok(receipt) => receipt,
            Err(error) if target == "all" => Receipt {
                id: String::new(),
                recipient,
                status: "failed".into(),
                error: Some(error),
            },
            Err(error) => return Err(error),
        });
    }
    let delivered = receipts
        .iter()
        .filter(|receipt| receipt.error.is_none())
        .count();
    let status = if delivered == receipts.len() {
        "queued"
    } else if delivered == 0 {
        "failed"
    } else {
        "partial"
    };
    Ok(SendReceipt {
        id: receipts
            .iter()
            .find(|receipt| receipt.error.is_none())
            .map(|receipt| receipt.id.clone())
            .unwrap_or_default(),
        recipient: target.into(),
        status: status.into(),
        receipts,
    })
}

async fn send_to_recipient(
    app: &AppHandle,
    mut message: Message,
    sender_run: Option<&(u64, Option<String>)>,
) -> Result<Receipt, String> {
    let state = app.state::<AppState>();
    let manager = &state.sub_agents;
    let mut recipient = if message.to == MAIN_AGENT_ID {
        None
    } else {
        let record = manager
            .registry
            .resolve(&message.root, &message.to)
            .ok_or_else(|| format!("Unknown agent '{}'", message.to))?;
        if !accepts_irc(&record) {
            return Err("Advisor sessions do not accept IRC messages".into());
        }
        Some(record)
    };
    loop {
        if let Some(record) = &recipient {
            recipient = Some(
                lifecycle::ensure_live(app, &message.root, &record.id, record.generation).await?,
            );
        }
        if !message.wake_relay
            && sender_run.is_some_and(|(generation, run)| {
                manager
                    .registry
                    .get(&message.root, &message.from)
                    .is_none_or(|record| {
                        record.generation != *generation
                            || record.run_id != *run
                            || record.status != AgentStatus::Running
                    })
            })
        {
            return Err("Sender run ended before delivery".into());
        }
        let delivery_slot = recipient
            .as_ref()
            .map(|record| manager.sessions.slot(record))
            .transpose()?;
        let recipient_control = delivery_slot.as_ref().map(|slot| {
            slot.control
                .lock()
                .unwrap_or_else(|error| error.into_inner())
        });
        if let (Some(record), Some(control)) = (&recipient, &recipient_control) {
            let current = manager
                .registry
                .get(&message.root, &record.id)
                .ok_or("Recipient disappeared before delivery")?;
            if control.killed
                || current.generation != record.generation
                || control.generation != record.generation
                || current.status == AgentStatus::Aborted
            {
                return Err("Recipient changed before delivery".into());
            }
            if current.status == AgentStatus::Parked {
                continue;
            }
            message.to = current.id;
        }
        {
            let mut inner = manager
                .irc
                .inner
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if message.asks_reply() {
                let debts = inner
                    .debts
                    .entry((message.root.clone(), message.from.clone()))
                    .or_default();
                debts.retain(|debt| debt.target != message.to);
                debts.push(Debt {
                    target: message.to.clone(),
                    sequence: message.sequence,
                    promised: false,
                });
            }
            inner.sent.insert(
                (
                    message.root.clone(),
                    message.from.clone(),
                    message.to.clone(),
                ),
                message.sequence,
            );
        }
        let receipt = Receipt {
            id: message.id.clone(),
            recipient: message.to.clone(),
            status: "queued".into(),
            error: None,
        };
        let observation = message.view(true);
        let _ = app.emit("subagent-irc", &observation);
        manager.irc.enqueue(message);
        drop(recipient_control);
        if let Some(record) = recipient {
            schedule_wake(app, &record);
        } else {
            super::delivery::on_main_run_ended(app, &observation.conversation_id);
        }
        return Ok(receipt);
    }
}

pub async fn collect_for_main(
    app: &AppHandle,
    root: &str,
    _run: &str,
    history: &[RuntimeMessage],
    admission: &mut kivio_agent_sdk::agent::InjectionAdmission,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    let mut claim = state.sub_agents.irc.claim(root, MAIN_AGENT_ID, |_| true);
    let inputs: Vec<RuntimeMessage> = claim
        .messages()
        .iter()
        .filter(|message| {
            !history.iter().any(|item| {
                item.source
                    .get("_subagent_irc_id")
                    .and_then(serde_json::Value::as_str)
                    == Some(message.id.as_str())
                    || item
                        .source
                        .get("_subagent_result_id")
                        .and_then(serde_json::Value::as_str)
                        == Some(format!("subagent-irc-{}", message.id).as_str())
                    || item
                        .source
                        .get("_ui_message_id")
                        .and_then(serde_json::Value::as_str)
                        == Some(format!("subagent-irc-{}", message.id).as_str())
            })
        })
        .filter_map(|message| RuntimeMessage::from_legacy(message.model_input()))
        .collect();
    let prepared = kivio_agent_sdk::agent::PreparedInjection::new(
        inputs.into_iter().map(super::session_input).collect(),
    )
    .map_err(|error| error.to_string())?;
    super::view::persist_incoming_batch(
        app,
        &claim
            .messages()
            .iter()
            .map(|message| message.view(false))
            .collect::<Vec<_>>(),
    )
    .await?;
    claim.commit();
    admission.committed(prepared);
    Ok(())
}

pub(crate) async fn persist_message(app: &AppHandle, message: &Message) -> Result<(), String> {
    super::view::persist_incoming(app, &message.view(false)).await
}

pub fn has_pending_main(manager: &super::SubAgentManager, root: &str) -> bool {
    manager.irc.has_pending_main(root)
}

pub(crate) fn has_interrupting(manager: &super::SubAgentManager, root: &str, owner: &str) -> bool {
    let parent = manager
        .registry
        .get(root, owner)
        .map(|record| record.parent_id);
    let inner = manager
        .irc
        .inner
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    [&inner.incoming, &inner.failed]
        .iter()
        .filter_map(|queue| queue.get(&(root.into(), owner.into())))
        .flatten()
        .any(|queued| {
            let message = &queued.message;
            !message.broadcast
                && (message.from == "@user"
                    || parent.as_ref() == Some(&message.from)
                    || (!message.progress
                        && manager.registry.is_ancestor(root, owner, &message.from)))
        })
}

pub(crate) async fn wait_for_interrupting(
    manager: &super::SubAgentManager,
    root: &str,
    owner: &str,
) {
    let mut changed = manager.irc.subscribe();
    while !has_interrupting(manager, root, owner) {
        if changed.changed().await.is_err() {
            return;
        }
    }
}

pub(crate) fn schedule_wake(app: &AppHandle, record: &AgentRecord) {
    let state = app.state::<AppState>();
    if state.sub_agents.user_interrupted(&record.root)
        || (!state.sub_agents.irc.has_pending(&record.root, &record.id)
            && !state
                .runtime_injections
                .has_waking_input(&record.child_conversation_id))
    {
        return;
    }
    let Some((current, slot)) = reserve_wake(&state.sub_agents, record) else {
        return;
    };
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let result = async {
            if app
                .state::<AppState>()
                .sub_agents
                .user_interrupted(&current.root)
                || with_current_wake(&slot, current.generation, |_| ()).is_none()
            {
                return Ok(WakeOutcome::Deferred);
            }
            let record = lifecycle::ensure_live_for_wake(&app, &current).await?;
            wake(&app, record).await
        }
        .await;
        let state = app.state::<AppState>();
        let Some(result) = with_current_wake(&slot, current.generation, |control| {
            control.wake_reserved = false;
            result.map_err(|error| {
                let messages = state
                    .sub_agents
                    .irc
                    .failed_handoff(&current.root, &current.id);
                (error, messages)
            })
        }) else {
            state
                .sub_agents
                .irc
                .inner
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .waking
                .remove(&(current.root.clone(), current.id.clone(), current.generation));
            return;
        };
        match result {
            Err((error, messages)) => {
                eprintln!("Cannot wake subagent {}: {error}", current.id);
                for message in messages.into_iter().filter(Message::asks_reply) {
                    let body = format!(
                        "Agent {} could not process the message: {error}",
                        current.id
                    );
                    let _ = Box::pin(send_message(
                        &app,
                        &current.root,
                        &current.id,
                        &format!("agent://{}", message.from),
                        &body,
                        Some(message.id),
                        true,
                    ))
                    .await;
                }
            }
            Ok(WakeOutcome::Handled) => schedule_wake(&app, &current),
            Ok(WakeOutcome::Deferred)
                if state.sub_agents.irc.has_pending(&current.root, &current.id) =>
            {
                schedule_wake(&app, &current);
            }
            Ok(WakeOutcome::Deferred) => {}
        }
    });
}

fn reserve_wake(
    manager: &super::SubAgentManager,
    record: &AgentRecord,
) -> Option<(AgentRecord, Arc<Slot>)> {
    let current = manager.registry.get(&record.root, &record.id)?;
    if !matches!(current.status, AgentStatus::Idle | AgentStatus::Parked)
        || current.generation != record.generation
    {
        return None;
    }
    let slot = manager.sessions.slot(&current).ok()?;
    {
        let mut control = slot
            .control
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if control.generation != current.generation
            || control.killed
            || control.wake_reserved
            || control.active_run.is_some()
        {
            return None;
        }
        control.wake_reserved = true;
    }
    Some((current, slot))
}

fn with_current_wake<T>(
    slot: &Slot,
    generation: u64,
    apply: impl FnOnce(&mut Control) -> T,
) -> Option<T> {
    let mut control = slot
        .control
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    (control.generation == generation && !control.killed && control.wake_reserved)
        .then(|| apply(&mut control))
}

enum WakeOutcome {
    Handled,
    Deferred,
}

async fn wake(app: &AppHandle, record: AgentRecord) -> Result<WakeOutcome, String> {
    let state = app.state::<AppState>();
    let slot = state.sub_agents.sessions.slot(&record)?;
    let mut guard = slot.worker.lock().await;
    let Some(mut claim) = with_current_wake(&slot, record.generation, |_| {
        state
            .sub_agents
            .irc
            .claim(&record.root, &record.id, |_| true)
    }) else {
        return Ok(WakeOutcome::Deferred);
    };
    if state.sub_agents.user_interrupted(&record.root)
        || (claim.messages().is_empty()
            && !state
                .runtime_injections
                .has_waking_input(&record.child_conversation_id))
    {
        return Ok(WakeOutcome::Deferred);
    }
    let worker = guard.as_mut().ok_or("Subagent session is unavailable")?;
    let mut saved = worker
        .contract
        .clone()
        .ok_or("Subagent contract is unavailable")?;
    if !saved.options.workpool_items.is_empty() {
        saved.options.workpool_items.clear();
        saved.options.completion = super::spawn::CompletionPolicy::Text;
    }
    let inputs = claim
        .messages()
        .iter()
        .map(|message| message.worker_input(&record.parent_id));
    worker.history.extend(inputs);
    super::session::append_context(&record.session_file, &worker.history).await?;
    if saved.plan_state.mode == crate::chat::types::AgentPlanMode::Plan {
        claim.commit();
        return Ok(WakeOutcome::Deferred);
    }
    let waking = claim.messages().to_vec();
    let owner_job = waking.iter().any(|message| {
        (message.from == record.parent_id && message.asks_reply()) || message.from == "@user"
    });
    claim.commit();
    let Some((job, mut cancel)) = with_current_wake(&slot, record.generation, |_| {
        let jobs = &state.sub_agents.jobs;
        let job = if owner_job {
            jobs.register(&record.root, &record.parent_id, &record.id, "IRC follow-up")
        } else {
            jobs.register_inline(&record.root, &record.parent_id, &record.id, "IRC follow-up")
        };
        state
            .sub_agents
            .irc
            .inner
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .waking
            .insert(
                (record.root.clone(), record.id.clone(), record.generation),
                waking,
            );
        job
    }) else {
        return Ok(WakeOutcome::Deferred);
    };
    drop(guard);
    let Some(permit) = state
        .sub_agents
        .admission
        .acquire_wake(app, &record.root, &record.parent_id, &mut cancel)
        .await
    else {
        state
            .sub_agents
            .jobs
            .settle(&job, Err("IRC follow-up cancelled before execution".into()));
        finish_turn(app, &record, "", Some("cancelled before execution"), true).await;
        return Ok(WakeOutcome::Deferred);
    };
    executor::run(
        app.clone(),
        executor::Request {
            options: saved.options,
            record,
            job_id: job,
            index: 0,
            definition: saved.agent_definition,
            task: saved.task,
            solution_space: saved.solution_space,
            context: saved.context,
            plan_state: saved.plan_state,
            mcp_tool_ids: saved.mcp_tool_ids,
            mcp_server_ids: saved.mcp_server_ids,
            approval_policy: saved.approval_policy,
            interactive_host: false,
            parent_run_id: String::new(),
            parent_tool_call_id: String::new(),
            parent_provider_id: saved.resolved_model.provider,
            parent_model: saved.resolved_model.model,
            cancel_rx: cancel,
            permit,
        },
    )
    .await;
    Ok(WakeOutcome::Handled)
}

pub(crate) async fn finish_turn(
    app: &AppHandle,
    record: &AgentRecord,
    output: &str,
    error: Option<&str>,
    aborted: bool,
) {
    let state = app.state::<AppState>();
    let waking = state
        .sub_agents
        .irc
        .inner
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .waking
        .remove(&(record.root.clone(), record.id.clone(), record.generation))
        .unwrap_or_default();
    let mut senders = std::collections::HashSet::new();
    for message in waking.into_iter().filter(Message::asks_reply) {
        if message.from == record.parent_id || !senders.insert(message.from.clone()) {
            continue;
        }
        let explicit = state
            .sub_agents
            .irc
            .inner
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .sent
            .get(&(record.root.clone(), record.id.clone(), message.from.clone()))
            .is_some_and(|sequence| *sequence > message.sequence);
        if explicit && error.is_none() && !aborted {
            continue;
        }
        let body = error
            .map(|error| {
                format!(
                    "Agent {} {}: {error}",
                    record.id,
                    if aborted { "was cancelled" } else { "failed" }
                )
            })
            .unwrap_or_else(|| output.to_string());
        if let Err(error) = Box::pin(send_message(
            app,
            &record.root,
            &record.id,
            &format!("agent://{}", message.from),
            &body,
            Some(message.id),
            true,
        ))
        .await
        {
            eprintln!("Cannot relay subagent outcome: {error}");
        }
    }
    state.sub_agents.irc.bump();
}

pub(crate) fn schedule_restored(
    app: &AppHandle,
    record: AgentRecord,
    job: String,
    mut cancel: watch::Receiver<bool>,
    turn: super::session::SavedTurn,
) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let state = app.state::<AppState>();
        let result = async {
            if *cancel.borrow() {
                return Err("Interrupted restore cancelled".into());
            }
            let live =
                lifecycle::ensure_live(&app, &record.root, &record.id, record.generation).await?;
            let slot = state.sub_agents.sessions.slot(&live)?;
            let saved = slot
                .worker
                .lock()
                .await
                .as_ref()
                .and_then(|worker| worker.contract.clone())
                .ok_or("Missing interrupted session contract")?;
            let permit = state
                .sub_agents
                .admission
                .acquire_wake(&app, &record.root, &record.parent_id, &mut cancel)
                .await
                .ok_or("Interrupted restore cancelled")?;
            executor::run(
                app.clone(),
                executor::Request {
                    options: saved.options,
                    record: live,
                    job_id: job.clone(),
                    index: 0,
                    definition: saved.agent_definition,
                    task: saved.task,
                    solution_space: saved.solution_space,
                    context: saved.context,
                    plan_state: saved.plan_state,
                    mcp_tool_ids: saved.mcp_tool_ids,
                    mcp_server_ids: saved.mcp_server_ids,
                    approval_policy: saved.approval_policy,
                    interactive_host: false,
                    parent_run_id: turn.parent_run_id,
                    parent_tool_call_id: turn.parent_tool_call_id,
                    parent_provider_id: saved.resolved_model.provider,
                    parent_model: saved.resolved_model.model,
                    cancel_rx: cancel,
                    permit,
                },
            )
            .await;
            Ok::<_, String>(())
        }
        .await;
        if let Err(error) = result {
            state.sub_agents.jobs.settle(&job, Err(error));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    fn message(sequence: u64, from: &str, to: &str, progress: bool) -> Message {
        Message {
            id: sequence.to_string(),
            root: "r".into(),
            from: from.into(),
            to: to.into(),
            body: "text".into(),
            sequence,
            reply_to: None,
            progress,
            broadcast: false,
            wake_relay: false,
            timestamp: 0,
        }
    }
    #[test]
    fn validates_urls_and_progress_presence() {
        assert_eq!(parse("agent://A?progress=false").unwrap(), ("A", true));
        assert!(parse("agent://A/path").is_err());
        assert!(parse("agent://all?progress").is_err());
    }

    #[tokio::test]
    async fn broadcast_attempts_later_peers_and_reports_each_failure() {
        let deliver = |recipient: String| async move {
            if recipient == "Stopped" {
                Err("Recipient changed before delivery".into())
            } else {
                Ok(Receipt {
                    id: format!("message-{recipient}"),
                    recipient,
                    status: "queued".into(),
                    error: None,
                })
            }
        };
        let result = deliver_targets(
            "all",
            ["First", "Stopped", "Last"].map(String::from).into(),
            deliver,
        )
        .await
        .unwrap();
        assert_eq!(result.status, "partial");
        assert_eq!(result.id, "message-First");
        assert_eq!(
            result
                .receipts
                .iter()
                .map(|receipt| (receipt.recipient.as_str(), receipt.status.as_str()))
                .collect::<Vec<_>>(),
            [
                ("First", "queued"),
                ("Stopped", "failed"),
                ("Last", "queued")
            ],
        );
        assert_eq!(
            result.receipts[1].error.as_deref(),
            Some("Recipient changed before delivery")
        );
        let failed = deliver_targets("all", vec!["Stopped".into()], deliver)
            .await
            .unwrap();
        assert_eq!(failed.status, "failed");
        assert!(deliver_targets("Stopped", vec!["Stopped".into()], deliver)
            .await
            .is_err());
    }
    #[test]
    fn failed_persistence_rolls_back_and_claims_consume_once() {
        let bus = Bus::default();
        bus.enqueue(message(1, "A", "B", false));
        {
            let claim = bus.claim("r", "B", |_| true);
            assert_eq!(claim.messages().len(), 1);
        }
        let mut claim = bus.claim("r", "B", |_| true);
        assert_eq!(claim.messages()[0].id, "1");
        claim.commit();
        drop(claim);
        assert!(!bus.has_pending("r", "B"));
    }

    #[test]
    fn empty_claim_does_not_notify_but_rollback_does() {
        let bus = Bus::default();
        let mut changed = bus.subscribe();
        drop(bus.claim("r", "B", |_| true));
        assert!(!changed.has_changed().unwrap());
        bus.enqueue(message(1, "A", "B", false));
        changed.borrow_and_update();
        drop(bus.claim("r", "B", |_| true));
        assert!(changed.has_changed().unwrap());
        assert!(bus.has_pending("r", "B"));
    }

    #[test]
    fn stale_wake_cannot_claim_move_or_clear_a_reopened_workers_mail() {
        let slot = Slot {
            control: Mutex::new(Control {
                generation: 2,
                wake_reserved: true,
                ..Control::default()
            }),
            worker: tokio::sync::Mutex::new(None),
        };
        let bus = Bus::default();
        bus.seed_message(message(1, "A", "B", false));
        assert!(with_current_wake(&slot, 1, |_| bus.claim("r", "B", |_| true)).is_none());
        assert!(with_current_wake(&slot, 1, |control| {
            control.wake_reserved = false;
            bus.failed_handoff("r", "B")
        })
        .is_none());
        assert!(bus.inner.lock().unwrap().failed.is_empty());
        let mut claim = with_current_wake(&slot, 2, |_| bus.claim("r", "B", |_| true)).unwrap();
        assert_eq!(claim.messages().len(), 1);
        claim.commit();
    }

    #[test]
    fn idle_and_parked_notifications_share_one_wake_reservation() {
        for status in [
            AgentStatus::Idle,
            AgentStatus::Parked,
            AgentStatus::Running,
            AgentStatus::Aborted,
        ] {
            let manager = super::super::SubAgentManager::default();
            let record = manager.registry.register(
                serde_json::from_value(json!({
                    "id":"Child", "root":"root", "parentId":"Main", "generation":0,
                    "displayName":"Child", "agentType":"task", "status":status,
                    "depth":1, "childConversationId":"child", "runId":null,
                    "sessionFile":"child.jsonl", "outputFile":"child.md",
                    "lastActivity":0, "activity":null, "published":true
                }))
                .unwrap(),
            );
            if matches!(status, AgentStatus::Running | AgentStatus::Aborted) {
                assert!(reserve_wake(&manager, &record).is_none());
                continue;
            }
            let (_, slot) = reserve_wake(&manager, &record).unwrap();
            assert!(reserve_wake(&manager, &record).is_none());
            with_current_wake(&slot, record.generation, |control| {
                control.wake_reserved = false;
                control.active_run = Some("user-run".into());
            })
            .unwrap();
            assert!(reserve_wake(&manager, &record).is_none());
            {
                let mut control = slot.control.lock().unwrap();
                control.active_run = None;
                control.killed = true;
            }
            assert!(reserve_wake(&manager, &record).is_none());
        }
    }

    #[test]
    fn human_and_parent_followups_are_instructions_while_peer_messages_remain_evidence() {
        for from in ["@user", "Parent"] {
            let input = message(1, from, "Child", false).worker_input("Parent");
            assert_eq!(input["role"], "user");
            assert!(!input["content"]
                .as_str()
                .unwrap()
                .contains("external evidence"));
            assert!(input["content"]
                .as_str()
                .unwrap()
                .starts_with("Message from"));
        }
        let peer = message(1, "Peer", "Child", false).worker_input("Parent");
        assert_eq!(peer["role"], "developer");
        assert!(peer["content"]
            .as_str()
            .unwrap()
            .contains("external evidence"));
        let mut broadcast = message(1, "Parent", "Child", false);
        broadcast.broadcast = true;
        assert_eq!(broadcast.worker_input("Parent")["role"], "developer");
    }
    #[test]
    fn progress_promises_a_reply_and_formal_reply_clears_debt() {
        let bus = Bus::default();
        bus.inner.lock().unwrap().debts.insert(
            ("r".into(), "A".into()),
            vec![Debt {
                target: "B".into(),
                sequence: 1,
                promised: false,
            }],
        );
        bus.enqueue(message(2, "B", "A", true));
        assert!(bus.pending_replies("r", "A")[0].promised);
        bus.enqueue(message(3, "B", "A", false));
        assert!(bus.pending_replies("r", "A").is_empty());
    }

    #[test]
    fn broadcast_reply_clears_question_with_or_without_progress() {
        for progress in [false, true] {
            let bus = Bus::default();
            bus.seed_message(message(1, "ChildA", "Main", false));
            if progress {
                bus.seed_message(message(2, "Main", "ChildA", true));
            }
            let pending = bus.pending_replies("r", "ChildA");
            assert_eq!(pending.len(), 1);
            assert_eq!(pending[0].target, "Main");
            assert_eq!(pending[0].promised, progress);

            let mut broadcast = message(3, "Main", "ChildA", false);
            broadcast.broadcast = true;
            bus.seed_message(broadcast);
            assert!(bus.pending_replies("r", "Main").is_empty());
            assert!(bus.pending_replies("r", "ChildA").is_empty());
        }
    }

    #[test]
    fn broadcast_reply_preserves_newer_and_unrelated_questions() {
        let bus = Bus::default();
        bus.seed_message(message(3, "ChildA", "Main", false));
        bus.seed_message(message(4, "ChildA", "ChildB", false));
        assert_eq!(bus.pending_replies("r", "ChildA").len(), 2);

        let mut old_broadcast = message(2, "Main", "ChildA", false);
        old_broadcast.broadcast = true;
        bus.seed_message(old_broadcast);
        assert_eq!(bus.pending_replies("r", "ChildA").len(), 2);

        bus.seed_message(message(5, "ChildC", "ChildA", false));
        assert_eq!(bus.pending_replies("r", "ChildA").len(), 2);

        let mut other_root = message(6, "Main", "ChildA", false);
        other_root.root = "other".into();
        other_root.broadcast = true;
        bus.seed_message(other_root);
        assert_eq!(bus.pending_replies("r", "ChildA").len(), 2);
        assert!(bus.pending_replies("other", "Main").is_empty());

        let mut unrelated_broadcast = message(7, "Main", "ChildB", false);
        unrelated_broadcast.broadcast = true;
        bus.seed_message(unrelated_broadcast);
        assert_eq!(bus.pending_replies("r", "ChildA").len(), 2);
        assert!(bus.pending_replies("r", "Main").is_empty());

        let mut answer = message(8, "Main", "ChildA", false);
        answer.broadcast = true;
        bus.seed_message(answer);
        let pending = bus.pending_replies("r", "ChildA");
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].target, "ChildB");
        assert!(bus.pending_replies("r", "Main").is_empty());
    }

    #[test]
    fn broadcast_answer_survives_claim_rollback_and_failed_handoff() {
        let bus = Bus::default();
        let mut aside = message(1, "Main", "ChildA", false);
        aside.broadcast = true;
        bus.seed_message(aside);
        bus.seed_message(message(2, "ChildA", "Main", false));
        bus.seed_message(message(3, "Main", "ChildA", true));
        let mut answer = message(4, "Main", "ChildA", false);
        answer.broadcast = true;
        bus.seed_message(answer);
        assert!(bus.pending_replies("r", "ChildA").is_empty());
        {
            let claim = bus.claim_wait("r", "ChildA", None);
            assert_eq!(claim.messages().len(), 1);
            assert_eq!(claim.messages()[0].sequence, 4);
        }
        bus.failed_handoff("r", "ChildA");
        let mut claim = bus.claim_wait("r", "ChildA", None);
        assert_eq!(claim.messages().len(), 1);
        assert_eq!(claim.messages()[0].sequence, 4);
        claim.commit();
        assert!(bus.claim_wait("r", "ChildA", None).messages().is_empty());
        let remaining = bus.claim("r", "ChildA", |_| true);
        assert_eq!(remaining.messages().len(), 2);
    }

    #[tokio::test]
    async fn descendant_question_releases_the_ancestor_barrier_but_peer_and_progress_do_not() {
        let manager = super::super::SubAgentManager::default();
        for (id, parent) in [("A", "Main"), ("Child", "A"), ("Peer", "Main")] {
            let record = serde_json::from_value(json!({"id":id,"root":"r","parentId":parent,"generation":0,"displayName":id,
                "agentType":"task","status":"running","depth":1,"childConversationId":format!("subagent-r-{id}"),
                "runId":null,"sessionFile":"A.jsonl","outputFile":"A.md","lastActivity":0,"activity":null,"published":true})).unwrap();
            manager.registry.register(record);
        }
        manager.irc.enqueue(message(1, "Peer", "A", false));
        manager.irc.enqueue(message(2, "Child", "A", true));
        assert!(!has_interrupting(&manager, "r", "A"));
        let pending = wait_for_interrupting(&manager, "r", "A");
        tokio::pin!(pending);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(1), &mut pending)
                .await
                .is_err()
        );
        manager.irc.enqueue(message(3, "Child", "A", false));
        tokio::time::timeout(std::time::Duration::from_millis(50), pending)
            .await
            .unwrap();
        manager.irc.claim("r", "A", |_| true).commit();
        let mut broadcast = message(4, "@user", "A", false);
        broadcast.broadcast = true;
        manager.irc.enqueue(broadcast);
        assert!(!has_interrupting(&manager, "r", "A"));
        manager.irc.enqueue(message(5, "@user", "A", false));
        assert!(has_interrupting(&manager, "r", "A"));
    }
}
