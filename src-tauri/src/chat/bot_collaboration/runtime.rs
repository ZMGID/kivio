//! Single writer over the state machine, plus dispatch through Chat send.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::{watch, Notify, Semaphore};

use crate::chat::types::{Conversation, ToolCallStatus};
use crate::state::AppState;

use super::machine::Collaboration;
use super::observe::classify_native_reply;
use super::store::{CommitError, Store};
use super::tools::is_coordination_mutation;
use super::types::{
    Actor, BroadcastReceipt, CollaborationSnapshot, Delivery, DeliveryStatus, Enqueue, Exchange,
    FinishReport, MemberConversationIdentity, MemberInput, PeerMessage, PendingAction, PendingKind,
    Room, RunFinish, CHANGED_EVENT, MAX_PARALLEL_DISPATCH,
};
use super::{
    bot_conversation_id, bot_id_from_conversation, identity_for_members, is_bot_conversation,
    is_room_conversation, validate_bot_id,
};

pub struct DispatchClaim {
    pub delivery: Delivery,
    pub cancel: watch::Receiver<bool>,
}

enum ExecuteEnd {
    Recorded,
    Finish(RunFinish),
}

pub struct BotCollaboration {
    inner: Mutex<Collaboration>,
    store: Store,
    wake: Notify,
    cancels: Mutex<HashMap<String, watch::Sender<bool>>>,
}

impl BotCollaboration {
    pub fn load(dir: PathBuf) -> Self {
        let store = Store::new(dir);
        let mut collab = match store.load() {
            Ok(None) => Collaboration::empty(),
            Ok(Some(loaded)) => Collaboration::from_disk(loaded.state, loaded.legacy),
            Err(err) => Collaboration::poisoned(err),
        };
        if collab.load_error().is_none() && collab.interrupt_inflight(now_secs()) {
            match store.commit(&collab.scoped_batch()) {
                Ok(()) => collab.clear_writes(),
                Err(CommitError::Durable(err)) => {
                    eprintln!(
                        "[bot-collaboration] 中断状态已进入事务，记录文件尚未全部替换：{err}"
                    );
                    if let Ok(Some(loaded)) = store.load() {
                        collab = Collaboration::from_disk(loaded.state, loaded.legacy);
                    }
                }
                Err(CommitError::NotDurable(err)) => {
                    eprintln!(
                        "[bot-collaboration] 中断状态没有写入，已停止协作调度，避免重放：{err}"
                    );
                    collab =
                        Collaboration::poisoned(format!("中断状态没有写入，已停止协作调度：{err}"));
                }
            }
        }
        Self {
            inner: Mutex::new(collab),
            store,
            wake: Notify::new(),
            cancels: Mutex::new(HashMap::new()),
        }
    }

    pub fn snapshot(&self) -> CollaborationSnapshot {
        self.lock().snapshot()
    }

    pub fn enqueue(&self, input: Enqueue, now: i64) -> Result<Delivery, String> {
        let delivery = self.with_mut(|collab| collab.enqueue(input, now))?;
        self.wake.notify_waiters();
        Ok(delivery)
    }

    pub fn record_unavailable(
        &self,
        input: Enqueue,
        reason: String,
        now: i64,
    ) -> Result<Delivery, String> {
        let delivery = self.with_mut(|collab| collab.record_unavailable(input, reason, now))?;
        self.wake.notify_waiters();
        Ok(delivery)
    }

    pub fn create_room(
        &self,
        title: &str,
        members: Vec<MemberInput>,
        now: i64,
    ) -> Result<Room, String> {
        self.with_mut(|collab| collab.create_room(title, members, now))
    }

    pub fn discard_room(&self, room_id: &str) -> Result<(), String> {
        self.with_mut(|collab| collab.discard_room(room_id))
    }

    pub fn set_members(
        &self,
        room_id: &str,
        members: Vec<MemberInput>,
        now: i64,
    ) -> Result<Room, String> {
        let room = self.with_mut(|collab| collab.set_members(room_id, members, now))?;
        self.wake.notify_waiters();
        Ok(room)
    }

    pub fn request_member_change(
        &self,
        room_id: &str,
        kind: PendingKind,
        bot_id: &str,
        bot_name: &str,
        now: i64,
    ) -> Result<PendingAction, String> {
        self.with_mut(|collab| collab.request_member_change(room_id, kind, bot_id, bot_name, now))
    }

    pub fn pause_room(&self, room_id: &str, now: i64) -> Result<Room, String> {
        self.with_mut(|collab| collab.pause_room(room_id, now))
    }

    pub fn continue_room(&self, room_id: &str, now: i64) -> Result<Room, String> {
        let room = self.with_mut(|collab| collab.continue_room(room_id, now))?;
        self.wake.notify_waiters();
        Ok(room)
    }

    pub fn rename_room(&self, room_id: &str, title: &str, now: i64) -> Result<Room, String> {
        self.with_mut(|collab| collab.rename_room(room_id, title, now))
    }

    pub fn broadcast(
        &self,
        from: Actor,
        room_id: &str,
        body: String,
        attachment_ids: Vec<String>,
        attachment_sources: Vec<String>,
        exchange_id: Option<String>,
        parent_delivery_id: Option<String>,
        now: i64,
    ) -> Result<BroadcastReceipt, String> {
        let receipt = self.with_mut(|collab| {
            collab.broadcast(
                from,
                room_id,
                body,
                attachment_ids,
                attachment_sources,
                exchange_id,
                parent_delivery_id,
                now,
            )
        })?;
        self.wake.notify_waiters();
        Ok(receipt)
    }

    pub fn align_member_thread(
        &self,
        room_id: &str,
        bot_id: &str,
        conversation_id: &str,
        now: i64,
    ) -> Result<Room, String> {
        self.with_mut(|collab| collab.align_member_thread(room_id, bot_id, conversation_id, now))
    }

    pub fn stop_room(&self, room_id: &str, now: i64) -> Result<Room, String> {
        let (room, signal) = self.with_mut(|collab| collab.stop_room(room_id, now))?;
        let cancels = self.lock_cancels();
        for id in signal {
            if let Some(sender) = cancels.get(&id) {
                let _ = sender.send(true);
            }
        }
        drop(cancels);
        self.wake.notify_waiters();
        Ok(room)
    }

    pub fn resolve(
        &self,
        action_id: &str,
        approve: bool,
        now: i64,
    ) -> Result<PendingAction, String> {
        let action = self.with_mut(|collab| collab.resolve(action_id, approve, now))?;
        self.wake.notify_waiters();
        Ok(action)
    }

    pub fn exchange(&self, id: &str) -> Result<Exchange, String> {
        self.lock().exchange(id)
    }

    pub fn room(&self, id: &str) -> Result<Room, String> {
        self.lock().room(id)
    }

    pub fn get_delivery(&self, id: &str) -> Option<Delivery> {
        self.lock().get_delivery(id)
    }

    pub fn inbound_for(
        &self,
        conversation_id: &str,
        delivery_id: Option<&str>,
    ) -> Result<Delivery, String> {
        self.lock().inbound_for(conversation_id, delivery_id)
    }

    pub fn active_inbound(&self, conversation_id: &str) -> Option<Delivery> {
        self.lock().active_inbound(conversation_id)
    }

    pub fn claim_next(
        &self,
        busy: &HashSet<String>,
        token: String,
        now: i64,
    ) -> Result<Option<Delivery>, String> {
        self.with_mut(|collab| Ok(collab.claim_next(busy, token, now)))
    }

    pub fn claim_for_dispatch(
        &self,
        busy: &HashSet<String>,
        now: i64,
    ) -> Result<Option<DispatchClaim>, String> {
        let token = uuid::Uuid::new_v4().to_string();
        let Some(delivery) = self.claim_next(busy, token, now)? else {
            return Ok(None);
        };
        let (sender, cancel) = watch::channel(false);
        self.lock_cancels().insert(delivery.id.clone(), sender);
        Ok(Some(DispatchClaim { delivery, cancel }))
    }

    pub fn begin_send(&self, id: &str, token: &str, now: i64) -> Result<bool, String> {
        self.with_mut(|collab| Ok(collab.begin_send(id, token, now)))
    }

    pub fn still_current(&self, id: &str, token: &str) -> bool {
        self.lock().still_current(id, token)
    }

    pub fn finish(
        &self,
        id: &str,
        token: &str,
        recipient_actor: Actor,
        outcome: RunFinish,
        now: i64,
    ) -> Result<FinishReport, String> {
        let report =
            self.with_mut(|collab| collab.finish(id, token, recipient_actor, outcome, now))?;
        self.wake.notify_waiters();
        Ok(report)
    }

    pub fn mark_unavailable(
        &self,
        id: &str,
        token: &str,
        reason: String,
        now: i64,
    ) -> Result<Delivery, String> {
        self.with_mut(|collab| collab.mark_unavailable(id, token, reason, now))
    }

    pub fn requeue_unadmitted(&self, id: &str, token: &str, now: i64) -> Result<bool, String> {
        let released = self.with_mut(|collab| Ok(collab.requeue_unadmitted(id, token, now)))?;
        if released {
            self.wake.notify_waiters();
        }
        Ok(released)
    }

    pub fn queued_conversation_ids(&self) -> Vec<String> {
        self.lock().queued_conversation_ids()
    }

    pub fn prompt_for(&self, conversation_id: &str) -> Option<String> {
        let delivery = self.lock().inbound_for(conversation_id, None).ok()?;
        if delivery.status != DeliveryStatus::Running {
            return None;
        }
        let from = match &delivery.from {
            Actor::User => "用户".to_string(),
            Actor::Bot { name, id, .. } => format!("{name}（{id}）"),
        };
        Some(format!(
            "这次回复来自同伴投递 {}，发送方是 {from}。使用当前助手自己的配置和记忆。不要把发送方的会话摘要当作自己的记忆，也不要代替其他 Bot 发言。敏感交接必须等用户确认。",
            delivery.id
        ))
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Collaboration> {
        self.inner.lock().unwrap_or_else(|err| err.into_inner())
    }

    fn lock_cancels(&self) -> std::sync::MutexGuard<'_, HashMap<String, watch::Sender<bool>>> {
        self.cancels.lock().unwrap_or_else(|err| err.into_inner())
    }

    fn with_mut<T>(
        &self,
        mutate: impl FnOnce(&mut Collaboration) -> Result<T, String>,
    ) -> Result<T, String> {
        let mut guard = self.lock();
        if let Some(err) = guard.load_error() {
            return Err(err.to_string());
        }
        let result = mutate(&mut guard);
        if result.is_err() {
            if guard.dirty {
                self.reload_into(&mut guard);
            }
            return result;
        }
        if !guard.dirty {
            return result;
        }
        let batch = guard.scoped_batch();
        match self.store.commit(&batch) {
            Ok(()) => {
                guard.clear_writes();
                result
            }
            Err(CommitError::Durable(err)) => {
                eprintln!("[bot-collaboration] 事务已记录，记录文件尚未全部替换：{err}");
                self.reload_into(&mut guard);
                result
            }
            Err(CommitError::NotDurable(err)) => {
                self.reload_into(&mut guard);
                Err(err)
            }
        }
    }

    fn reload_into(&self, guard: &mut Collaboration) {
        match self.store.load() {
            Ok(None) => *guard = Collaboration::empty(),
            Ok(Some(loaded)) => *guard = Collaboration::from_disk(loaded.state, loaded.legacy),
            Err(err) => *guard = Collaboration::poisoned(err),
        }
    }
}

pub fn now_secs() -> i64 {
    chrono::Local::now().timestamp()
}

pub fn try_service(app: &AppHandle) -> Result<tauri::State<'_, BotCollaboration>, String> {
    app.try_state::<BotCollaboration>()
        .ok_or_else(|| "Bot 协作尚未初始化".to_string())
}

/// Durable delivery records for read-time correlation. Empty when collaboration
/// is not loaded. Does not write history.
pub fn recorded_deliveries(app: &AppHandle, conversation_id: &str) -> Vec<Delivery> {
    try_service(app)
        .ok()
        .map(|service| service.lock().deliveries_for_conversation(conversation_id))
        .unwrap_or_default()
}

pub fn publish(app: &AppHandle) {
    let Ok(service) = try_service(app) else {
        return;
    };
    let _ = app.emit(CHANGED_EVENT, service.snapshot());
}

pub fn require_bot(app: &AppHandle, bot_id: &str) -> Result<MemberInput, String> {
    let bot_id = validate_bot_id(bot_id)?.to_string();
    let conversation_id = bot_conversation_id(&bot_id);
    let conversation =
        crate::chat::storage::load_conversation(app, &conversation_id).map_err(|err| {
            format!("Bot「{bot_id}」不可用：没有可投递的会话 {conversation_id}（{err}）")
        })?;
    Ok(MemberInput {
        bot_id,
        name: bot_name(&conversation),
    })
}

/// Stable assistant for a private Bot chat or one group-member thread.
/// `Ok(None)` is an ordinary chat. A member thread that is not exactly one Bot
/// is `Err`, so callers do not fall through to global memory.
pub fn member_assistant_id(
    app: &AppHandle,
    conversation_id: &str,
) -> Result<Option<String>, String> {
    if let Some(id) = crate::chat::bot_profile::assistant_id_from_conversation(conversation_id) {
        return Ok(Some(id.to_string()));
    }
    let encoded = crate::chat::bot_profile::encoded_member_bot(conversation_id)?;
    let recorded = recorded_member_bot(app, conversation_id)?;
    match (encoded, recorded) {
        (Some(encoded), Some(recorded)) if encoded != recorded => Err(format!(
            "群成员会话「{conversation_id}」的 Bot 不一致（{encoded} / {recorded}），没有改用全局记忆"
        )),
        (Some(id), _) | (None, Some(id)) => Ok(Some(id)),
        (None, None) => Ok(None),
    }
}

fn recorded_member_bot(app: &AppHandle, conversation_id: &str) -> Result<Option<String>, String> {
    let Some(service) = try_service(app).ok() else {
        return Ok(None);
    };
    let snapshot = service.snapshot();
    let rows = snapshot.rooms.iter().flat_map(|room| {
        room.members
            .iter()
            .map(|member| (member.conversation_id.as_str(), member.bot_id.as_str()))
    });
    crate::chat::bot_profile::assistant_id_from_member_rows(conversation_id, rows)
}

pub fn member_conversation_identity(
    app: &AppHandle,
    conversation_id: &str,
) -> Result<Option<MemberConversationIdentity>, String> {
    let conversation_id = conversation_id.trim();
    if conversation_id.is_empty() {
        return Ok(None);
    }
    let assistant_id = member_assistant_id(app, conversation_id)?;
    if let Some(assistant_id) = assistant_id {
        let recorded = try_service(app)
            .ok()
            .and_then(|service| identity_for_members(conversation_id, &service.snapshot().rooms));
        let name = recorded
            .as_ref()
            .map(|identity| identity.name.clone())
            .or_else(|| {
                crate::chat::storage::load_conversation(app, conversation_id)
                    .ok()
                    .map(|conversation| bot_name(&conversation))
            })
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| assistant_id.clone());
        let room_id = recorded
            .and_then(|identity| identity.room_id)
            .or_else(|| super::member_room_id(conversation_id).map(str::to_string));
        return Ok(Some(MemberConversationIdentity {
            bot_id: assistant_id.clone(),
            name,
            conversation_id: conversation_id.to_string(),
            room_id,
            assistant_id: Some(assistant_id),
        }));
    }
    let Some(bot_id) = bot_id_from_conversation(conversation_id) else {
        return Ok(None);
    };
    let name = crate::chat::storage::load_conversation(app, conversation_id)
        .ok()
        .map(|conversation| bot_name(&conversation))
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| bot_id.to_string());
    Ok(Some(MemberConversationIdentity {
        bot_id: bot_id.to_string(),
        name,
        conversation_id: conversation_id.to_string(),
        room_id: None,
        assistant_id: None,
    }))
}

pub fn collaboration_conversation(app: &AppHandle, conversation_id: &str) -> bool {
    match member_assistant_id(app, conversation_id) {
        Ok(Some(_)) | Err(_) => true,
        Ok(None) => {
            is_bot_conversation(conversation_id)
                || try_service(app).ok().is_some_and(|service| {
                    is_room_conversation(conversation_id, &service.snapshot().rooms)
                })
        }
    }
}

pub fn actor_for_conversation(app: &AppHandle, conversation_id: &str) -> Result<Actor, String> {
    if let Some(identity) = member_conversation_identity(app, conversation_id)? {
        if bot_id_from_conversation(conversation_id).is_some() {
            let conversation = crate::chat::storage::load_conversation(app, conversation_id)?;
            return Ok(actor_for_bot(
                app,
                &identity.bot_id,
                bot_name(&conversation),
            ));
        }
        if crate::chat::bot_profile::assistant_id_from_member_conversation(conversation_id)
            .is_some()
        {
            if let Ok(conversation) = crate::chat::storage::load_conversation(app, conversation_id)
            {
                return Ok(actor_for_bot(
                    app,
                    &identity.bot_id,
                    bot_name(&conversation),
                ));
            }
        }
        return Ok(actor_for_bot(app, &identity.bot_id, identity.name));
    }
    if collaboration_conversation(app, conversation_id) {
        return Ok(Actor::User);
    }
    Err("只有 Bot 会话可以投递给同伴".into())
}

/// Capture the authoritative profile once, before creating a historical actor.
/// Legacy Bots without an assistant profile safely keep their name/default avatar.
pub(super) fn actor_for_bot(app: &AppHandle, bot_id: &str, name: String) -> Actor {
    crate::chat::storage::get_assistant(app, bot_id)
        .map(|assistant| Actor::from_assistant(&assistant))
        .unwrap_or_else(|_| Actor::bot(bot_id, name))
}

fn bot_name(conversation: &Conversation) -> String {
    conversation
        .assistant_snapshot
        .as_ref()
        .map(|snapshot| snapshot.name.trim().to_string())
        .filter(|name| !name.is_empty())
        .or_else(|| bot_id_from_conversation(&conversation.id).map(str::to_string))
        .unwrap_or_else(|| conversation.id.clone())
}

pub fn recipient_busy(app: &AppHandle, conversation_id: &str) -> bool {
    let runtime_busy = app
        .try_state::<AppState>()
        .is_some_and(|state| state.chat_runtime().has_active_reply(conversation_id));
    let scheduled_busy = app
        .try_state::<crate::scheduled_tasks::ScheduledTasks>()
        .is_some_and(|tasks| tasks.is_scheduled_conversation_busy(conversation_id));
    runtime_busy || scheduled_busy
}

pub fn spawn(app: AppHandle) {
    if app.try_state::<BotCollaboration>().is_none() {
        eprintln!("[bot-collaboration] spawn skipped: state not managed");
        return;
    }
    tauri::async_runtime::spawn(async move {
        let slots = Arc::new(Semaphore::new(MAX_PARALLEL_DISPATCH));
        loop {
            let Ok(service) = try_service(&app) else {
                break;
            };
            let notified = service.wake.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let Ok(permit) = slots.clone().acquire_owned().await else {
                break;
            };
            match claim_one(&app) {
                ClaimTick::Claimed(claim) => {
                    let app = app.clone();
                    tauri::async_runtime::spawn(async move {
                        let _permit = permit;
                        run_claimed(&app, claim).await;
                    });
                }
                ClaimTick::Idle => {
                    drop(permit);
                    tokio::select! {
                        _ = notified => {}
                        _ = tokio::time::sleep(Duration::from_secs(2)) => {}
                    }
                }
                ClaimTick::Error(err) => {
                    drop(permit);
                    eprintln!("[bot-collaboration] {err}");
                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
            }
        }
    });
}

enum ClaimTick {
    Claimed(DispatchClaim),
    Idle,
    Error(String),
}

fn claim_one(app: &AppHandle) -> ClaimTick {
    let service = match try_service(app) {
        Ok(service) => service,
        Err(err) => return ClaimTick::Error(err),
    };
    let busy = service
        .queued_conversation_ids()
        .into_iter()
        .filter(|id| recipient_busy(app, id))
        .collect::<HashSet<_>>();
    match service.claim_for_dispatch(&busy, now_secs()) {
        Ok(Some(claim)) => {
            publish(app);
            ClaimTick::Claimed(claim)
        }
        Ok(None) => ClaimTick::Idle,
        Err(err) => ClaimTick::Error(err),
    }
}

async fn run_claimed(app: &AppHandle, claim: DispatchClaim) {
    let service = match try_service(app) {
        Ok(service) => service,
        Err(err) => {
            eprintln!("[bot-collaboration] {err}");
            return;
        }
    };
    let id = claim.delivery.id.clone();
    let token = claim.delivery.claim_token.clone().unwrap_or_default();
    let recipient_actor = actor_for_bot(
        app,
        &claim.delivery.to_bot_id,
        claim.delivery.to_name.clone(),
    );
    let end = execute_delivery(app, claim).await;
    if let ExecuteEnd::Finish(outcome) = end {
        if let Err(err) = service.finish(&id, &token, recipient_actor, outcome, now_secs()) {
            eprintln!("[bot-collaboration] 写入运行结果失败：{err}");
        }
    }
    publish(app);
}

async fn ensure_recipient_conversation(app: &AppHandle, delivery: &Delivery) -> Result<(), String> {
    let conversation_id = &delivery.to_conversation_id;
    if crate::chat::storage::load_conversation(app, conversation_id).is_ok() {
        return Ok(());
    }
    let Some(room_id) = delivery.room_id.as_deref() else {
        return crate::chat::storage::load_conversation(app, conversation_id).map(|_| ());
    };
    if crate::chat::bot_profile::assistant_id_from_member_conversation(conversation_id).is_none() {
        return crate::chat::storage::load_conversation(app, conversation_id).map(|_| ());
    }
    let room = try_service(app)?.room(room_id)?;
    let member = room
        .members
        .iter()
        .find(|member| member.bot_id == delivery.to_bot_id)
        .cloned()
        .ok_or_else(|| format!("Bot「{}」不是房间「{room_id}」的成员", delivery.to_bot_id))?;
    if member.conversation_id != *conversation_id {
        return Err(format!(
            "群成员会话「{conversation_id}」与房间记录「{}」不一致",
            member.conversation_id
        ));
    }
    super::flow::open_member_thread(app, &room, &member).await?;
    crate::chat::storage::load_conversation(app, conversation_id).map(|_| ())
}

async fn execute_delivery(app: &AppHandle, claim: DispatchClaim) -> ExecuteEnd {
    let delivery = claim.delivery;
    let token = delivery.claim_token.clone().unwrap_or_default();
    let conversation_id = delivery.to_conversation_id.clone();
    if let Err(err) = ensure_recipient_conversation(app, &delivery).await {
        let reason = format!(
            "Bot「{}」不可用：没有可投递的会话 {conversation_id}（{err}）",
            delivery.to_bot_id
        );
        if let Ok(service) = try_service(app) {
            let _ = service.mark_unavailable(&delivery.id, &token, reason, now_secs());
        }
        return ExecuteEnd::Recorded;
    }
    let peer = PeerMessage::from(&delivery);
    let content = delivery.body.clone();
    let prepared = match super::flow::prepare_delivery_attachments(app, &delivery) {
        Ok(prepared) => prepared,
        Err(message) => {
            return ExecuteEnd::Finish(RunFinish::Failed { message });
        }
    };
    let attachment_paths = prepared.paths.clone();
    let text_attachments = prepared.text.clone();
    let id = delivery.id.clone();
    let service = match try_service(app) {
        Ok(service) => service,
        Err(_) => {
            return ExecuteEnd::Finish(RunFinish::Failed {
                message: "Bot 协作尚未初始化".into(),
            })
        }
    };
    let on_send_ready = || match service.begin_send(&id, &token, now_secs()) {
        Ok(true) => {
            publish(app);
            true
        }
        Ok(false) => false,
        Err(err) => {
            eprintln!("[bot-collaboration] 运行状态没有写入，未开始发送：{err}");
            false
        }
    };
    let on_user_message_saved = || {};
    let on_send_finished = || {
        service.lock_cancels().remove(&id);
    };
    let send = crate::chat::commands::send::send_user_message_when_idle(
        app,
        &conversation_id,
        content,
        claim.cancel,
        &on_send_ready,
        &on_user_message_saved,
        &on_send_finished,
        attachment_paths,
        Some(text_attachments),
        Some(peer),
    )
    .await;
    drop(prepared);
    match send {
        Ok(crate::chat::commands::send::QueuedSendOutcome::Cancelled) => {
            ExecuteEnd::Finish(RunFinish::Cancelled {
                message: "等待投递时已取消，未发送".into(),
            })
        }
        Ok(crate::chat::commands::send::QueuedSendOutcome::Sent) => {
            match crate::chat::storage::load_conversation(app, &conversation_id) {
                Ok(conversation) => ExecuteEnd::Finish(observe_conversation(&conversation)),
                Err(err) => ExecuteEnd::Finish(RunFinish::Failed {
                    message: format!("运行结束但读不到会话：{err}"),
                }),
            }
        }
        Err(message) => ExecuteEnd::Finish(RunFinish::Failed { message }),
    }
}

pub fn observe_conversation(conversation: &Conversation) -> RunFinish {
    let Some(message) = conversation.messages.last() else {
        return RunFinish::Failed {
            message: format!("运行结束但没有消息，{}", super::types::NO_FABRICATED_REPLY),
        };
    };
    classify_native_reply(
        &message.role,
        &message.content,
        message.stream_outcome.as_deref(),
        turn_spent_coordination_budget(conversation),
    )
}

/// A successful peer, mention, handoff, or broadcast anywhere after the latest
/// user message already continued this exchange. The final assistant message
/// often has no tool call, so looking only there would auto-send a second hop.
fn turn_spent_coordination_budget(conversation: &Conversation) -> bool {
    let start = conversation
        .messages
        .iter()
        .rposition(|message| message.role == "user")
        .map(|index| index + 1)
        .unwrap_or(0);
    conversation.messages[start..].iter().any(|message| {
        message.tool_calls.iter().any(|call| {
            call.status == ToolCallStatus::Success && is_coordination_mutation(&call.name)
        })
    })
}
