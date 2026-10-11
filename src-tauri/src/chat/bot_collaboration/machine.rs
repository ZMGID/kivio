//! Pure collaboration state. Persistence and the native run live outside.

use std::collections::HashSet;

use uuid::Uuid;

use super::store::RecordBatch;
use super::types::{
    bot_conversation_id, normalize_attachment_ids, normalize_room_title, validate_bot_id, Actor,
    BroadcastReceipt, CollaborationSnapshot, Delivery, DeliveryKind, DeliveryStatus, DiskState,
    Enqueue, Exchange, ExchangeEntry, FinishReport, LogKind, MemberInput, MemberState,
    PendingAction, PendingKind, PendingStatus, Receipt, ReceiptState, Room, RoomLogEntry,
    RoomMember, RoomStatus, RunFinish, GROUP_STOP_REASON, INTERRUPT_REASON, MAX_COORDINATION_DEPTH,
    MAX_DELIVERIES_PER_EXCHANGE, MAX_ROOM_BOTS,
};

#[derive(Debug, Clone, Default)]
struct ScopedWrites {
    deliveries: HashSet<String>,
    rooms: HashSet<String>,
    exchanges: HashSet<String>,
    pending: HashSet<String>,
    delete_deliveries: HashSet<String>,
    delete_rooms: HashSet<String>,
    delete_exchanges: HashSet<String>,
    delete_pending: HashSet<String>,
}

const MAX_LOG: usize = 200;
const MAX_ENTRIES: usize = 128;

#[derive(Debug, Clone)]
pub struct Collaboration {
    deliveries: Vec<Delivery>,
    rooms: Vec<Room>,
    exchanges: Vec<Exchange>,
    pending: Vec<PendingAction>,
    load_error: Option<String>,
    pub dirty: bool,
    /// Legacy `collaboration.json` still holds records that have no per-id file.
    pub force_full: bool,
    writes: ScopedWrites,
}

impl Collaboration {
    pub fn deliveries_for_conversation(&self, conversation_id: &str) -> Vec<Delivery> {
        self.deliveries
            .iter()
            .filter(|delivery| delivery.to_conversation_id == conversation_id)
            .cloned()
            .collect()
    }

    pub fn empty() -> Self {
        Self {
            deliveries: Vec::new(),
            rooms: Vec::new(),
            exchanges: Vec::new(),
            pending: Vec::new(),
            load_error: None,
            dirty: false,
            force_full: false,
            writes: ScopedWrites::default(),
        }
    }

    pub fn poisoned(error: String) -> Self {
        Self {
            load_error: Some(error),
            ..Self::empty()
        }
    }

    pub fn from_disk(disk: DiskState, legacy: bool) -> Self {
        Self {
            deliveries: disk.deliveries,
            rooms: disk.rooms,
            exchanges: disk.exchanges,
            pending: disk.pending,
            load_error: None,
            dirty: false,
            force_full: legacy,
            writes: ScopedWrites::default(),
        }
    }

    pub fn scoped_batch(&self) -> RecordBatch {
        if self.force_full
            || (self.dirty
                && self.writes.deliveries.is_empty()
                && self.writes.rooms.is_empty()
                && self.writes.exchanges.is_empty()
                && self.writes.pending.is_empty()
                && self.writes.delete_deliveries.is_empty()
                && self.writes.delete_rooms.is_empty()
                && self.writes.delete_exchanges.is_empty()
                && self.writes.delete_pending.is_empty())
        {
            return RecordBatch {
                put_deliveries: self.deliveries.clone(),
                put_rooms: self.rooms.clone(),
                put_exchanges: self.exchanges.clone(),
                put_pending: self.pending.clone(),
                replaces_legacy: self.force_full,
                ..RecordBatch::default()
            };
        }
        RecordBatch {
            put_deliveries: self
                .deliveries
                .iter()
                .filter(|delivery| self.writes.deliveries.contains(&delivery.id))
                .cloned()
                .collect(),
            put_rooms: self
                .rooms
                .iter()
                .filter(|room| self.writes.rooms.contains(&room.id))
                .cloned()
                .collect(),
            put_exchanges: self
                .exchanges
                .iter()
                .filter(|exchange| self.writes.exchanges.contains(&exchange.id))
                .cloned()
                .collect(),
            put_pending: self
                .pending
                .iter()
                .filter(|action| self.writes.pending.contains(&action.id))
                .cloned()
                .collect(),
            delete_deliveries: self.writes.delete_deliveries.iter().cloned().collect(),
            delete_rooms: self.writes.delete_rooms.iter().cloned().collect(),
            delete_exchanges: self.writes.delete_exchanges.iter().cloned().collect(),
            delete_pending: self.writes.delete_pending.iter().cloned().collect(),
            replaces_legacy: false,
        }
    }

    pub fn clear_writes(&mut self) {
        self.dirty = false;
        self.force_full = false;
        self.writes = ScopedWrites::default();
    }

    fn remember_delivery(&mut self, id: &str) {
        self.dirty = true;
        self.writes.deliveries.insert(id.to_string());
    }

    fn remember_room(&mut self, id: &str) {
        self.dirty = true;
        self.writes.rooms.insert(id.to_string());
    }

    fn remember_exchange(&mut self, id: &str) {
        self.dirty = true;
        self.writes.exchanges.insert(id.to_string());
    }

    fn remember_pending(&mut self, id: &str) {
        self.dirty = true;
        self.writes.pending.insert(id.to_string());
    }

    fn forget_room(&mut self, id: &str) {
        self.dirty = true;
        self.writes.rooms.remove(id);
        self.writes.delete_rooms.insert(id.to_string());
    }

    fn forget_pending(&mut self, id: &str) {
        self.dirty = true;
        self.writes.pending.remove(id);
        self.writes.delete_pending.insert(id.to_string());
    }

    pub fn snapshot(&self) -> CollaborationSnapshot {
        CollaborationSnapshot {
            deliveries: self.deliveries.clone(),
            rooms: self.rooms.clone(),
            exchanges: self.exchanges.clone(),
            pending: self.pending.clone(),
            error: self.load_error.clone(),
        }
    }

    pub fn load_error(&self) -> Option<&str> {
        self.load_error.as_deref()
    }

    /// Claimed and running work became a side effect that must not run again.
    pub fn interrupt_inflight(&mut self, now: i64) -> bool {
        let mut changed = false;
        let mut notes = Vec::new();
        for delivery in &mut self.deliveries {
            if matches!(
                delivery.status,
                DeliveryStatus::Claimed | DeliveryStatus::Running
            ) {
                delivery.status = DeliveryStatus::Interrupted;
                delivery.error = Some(INTERRUPT_REASON.to_string());
                delivery.finished_at = Some(now);
                delivery.updated_at = now;
                delivery.admitted = true;
                delivery.receipts.push(Receipt {
                    at: now,
                    state: ReceiptState::Interrupted,
                    detail: Some(INTERRUPT_REASON.to_string()),
                });
                notes.push((
                    delivery.id.clone(),
                    delivery.exchange_id.clone(),
                    delivery.entry_id.clone(),
                ));
                changed = true;
            }
        }
        for (delivery_id, exchange_id, entry_id) in notes {
            self.remember_delivery(&delivery_id);
            self.set_entry_status(&exchange_id, &entry_id, DeliveryStatus::Interrupted);
        }
        changed
    }

    pub fn get_delivery(&self, id: &str) -> Option<Delivery> {
        self.deliveries
            .iter()
            .find(|delivery| delivery.id == id)
            .cloned()
    }

    pub fn inbound_for(
        &self,
        conversation_id: &str,
        delivery_id: Option<&str>,
    ) -> Result<Delivery, String> {
        if let Some(id) = delivery_id {
            let delivery = self.get_delivery(id).ok_or("没有这次投递")?;
            if delivery.to_conversation_id != conversation_id {
                return Err("这次投递不是发给当前 Bot 的".into());
            }
            return Ok(delivery);
        }
        self.active_inbound(conversation_id)
            .ok_or_else(|| "当前没有待回复的同伴投递".to_string())
    }

    /// The delivery this conversation is executing now. Model sends continue it
    /// instead of opening an exchange at depth zero.
    pub fn active_inbound(&self, conversation_id: &str) -> Option<Delivery> {
        self.deliveries
            .iter()
            .rev()
            .find(|delivery| {
                delivery.to_conversation_id == conversation_id
                    && matches!(
                        delivery.status,
                        DeliveryStatus::Running | DeliveryStatus::Claimed
                    )
            })
            .cloned()
    }

    pub fn exchange(&self, id: &str) -> Result<Exchange, String> {
        self.exchanges
            .iter()
            .find(|exchange| exchange.id == id)
            .cloned()
            .ok_or_else(|| "没有这次交流".into())
    }

    pub fn room(&self, id: &str) -> Result<Room, String> {
        self.rooms
            .iter()
            .find(|room| room.id == id)
            .cloned()
            .ok_or_else(|| "没有这个协作房间".into())
    }

    pub fn queued_conversation_ids(&self) -> Vec<String> {
        self.deliveries
            .iter()
            .filter(|delivery| delivery.status == DeliveryStatus::Queued)
            .map(|delivery| delivery.to_conversation_id.clone())
            .collect()
    }

    pub fn create_room(
        &mut self,
        title: &str,
        members: Vec<MemberInput>,
        now: i64,
    ) -> Result<Room, String> {
        let id = new_id("room");
        let members = normalize_members(&id, &members, now)?;
        let title = {
            let trimmed = title.trim();
            if trimmed.is_empty() {
                "协作房间".to_string()
            } else {
                normalize_room_title(trimmed)?
            }
        };
        let room = Room {
            id,
            title,
            status: RoomStatus::Active,
            cycle: 1,
            members,
            log: Vec::new(),
            created_at: now,
            updated_at: now,
        };
        self.rooms.push(room.clone());
        self.remember_room(&room.id);
        Ok(room)
    }

    /// Drops a room that never received a delivery. Used when member-thread
    /// creation fails after `create_room` has already been saved.
    pub fn discard_room(&mut self, room_id: &str) -> Result<(), String> {
        if self
            .deliveries
            .iter()
            .any(|delivery| delivery.room_id.as_deref() == Some(room_id))
        {
            return Err(format!("房间「{room_id}」已有投递，不能撤回"));
        }
        let index = self
            .rooms
            .iter()
            .position(|room| room.id == room_id)
            .ok_or_else(|| format!("找不到房间「{room_id}」"))?;
        self.rooms.remove(index);
        let removed_pending: Vec<String> = self
            .pending
            .iter()
            .filter(|action| action.room_id.as_deref() == Some(room_id))
            .map(|action| action.id.clone())
            .collect();
        self.pending
            .retain(|action| action.room_id.as_deref() != Some(room_id));
        self.forget_room(room_id);
        for id in removed_pending {
            self.forget_pending(&id);
        }
        Ok(())
    }

    pub fn set_members(
        &mut self,
        room_id: &str,
        members: Vec<MemberInput>,
        now: i64,
    ) -> Result<Room, String> {
        let members = normalize_members(room_id, &members, now)?;
        let removed = {
            let room = self.room_mut(room_id)?;
            let removed: Vec<String> = room
                .members
                .iter()
                .filter(|member| !members.iter().any(|next| next.bot_id == member.bot_id))
                .map(|member| member.bot_id.clone())
                .collect();
            room.members = members;
            room.updated_at = now;
            removed
        };
        let room_id = room_id.to_string();
        self.push_log(
            &room_id,
            Actor::User,
            LogKind::Membership,
            "成员已更新",
            None,
            now,
        );
        for bot_id in removed {
            self.cancel_queued_for_member(&room_id, &bot_id, now);
        }
        self.remember_room(&room_id);
        self.room(&room_id)
    }

    pub fn request_member_change(
        &mut self,
        room_id: &str,
        kind: PendingKind,
        bot_id: &str,
        bot_name: &str,
        now: i64,
    ) -> Result<PendingAction, String> {
        if !matches!(kind, PendingKind::AddMember | PendingKind::RemoveMember) {
            return Err("成员变更类型无效".into());
        }
        let bot_id = validate_bot_id(bot_id)?.to_string();
        self.ensure_room_operable(room_id)?;
        if kind == PendingKind::RemoveMember && !self.is_member(room_id, &bot_id) {
            return Err(format!("Bot「{bot_id}」不是这个房间的成员"));
        }
        if kind == PendingKind::AddMember && self.is_member(room_id, &bot_id) {
            return Err(format!("Bot「{bot_id}」已经在房间里"));
        }
        if kind == PendingKind::AddMember {
            member_thread_id(&bot_id, room_id)?;
        }
        let action = PendingAction {
            id: new_id("act"),
            room_id: Some(room_id.to_string()),
            kind,
            bot_id: bot_id.clone(),
            bot_name: bot_name.trim().to_string(),
            delivery_id: None,
            status: PendingStatus::AwaitingUser,
            created_at: now,
            resolved_at: None,
        };
        self.pending.push(action.clone());
        self.remember_pending(&action.id);
        self.push_log(
            room_id,
            Actor::User,
            LogKind::Membership,
            &format!("等待用户确认成员变更：{bot_id}"),
            None,
            now,
        );
        Ok(action)
    }

    pub fn pause_room(&mut self, room_id: &str, now: i64) -> Result<Room, String> {
        let id = {
            let room = self.room_mut(room_id)?;
            if room.status == RoomStatus::Stopped {
                return Err("已停止的群组不能暂停".into());
            }
            room.status = RoomStatus::Paused;
            room.updated_at = now;
            room.id.clone()
        };
        self.push_log(&id, Actor::User, LogKind::Status, "群组已暂停", None, now);
        self.room(&id)
    }

    pub fn continue_room(&mut self, room_id: &str, now: i64) -> Result<Room, String> {
        let id = {
            let room = self.room_mut(room_id)?;
            room.status = RoomStatus::Active;
            room.updated_at = now;
            room.id.clone()
        };
        self.push_log(&id, Actor::User, LogKind::Status, "群组已继续", None, now);
        self.room(&id)
    }

    /// Stops this room's cycle only. Admitted runs finish, but do not chain.
    pub fn stop_room(&mut self, room_id: &str, now: i64) -> Result<(Room, Vec<String>), String> {
        if self.room(room_id)?.status == RoomStatus::Stopped {
            return Ok((self.room(room_id)?, Vec::new()));
        }
        let id = {
            let room = self.room_mut(room_id)?;
            room.status = RoomStatus::Stopped;
            room.cycle = room.cycle.saturating_add(1);
            room.updated_at = now;
            room.id.clone()
        };
        let mut signal = Vec::new();
        let mut cancelled = Vec::new();
        for delivery in &mut self.deliveries {
            if delivery.room_id.as_deref() != Some(id.as_str()) {
                continue;
            }
            let waiting = matches!(
                delivery.status,
                DeliveryStatus::Queued | DeliveryStatus::AwaitingUser | DeliveryStatus::Claimed
            ) && !delivery.admitted;
            if !waiting {
                continue;
            }
            if delivery.status == DeliveryStatus::Claimed {
                signal.push(delivery.id.clone());
            }
            delivery.status = DeliveryStatus::Cancelled;
            delivery.error = Some(GROUP_STOP_REASON.to_string());
            delivery.finished_at = Some(now);
            delivery.updated_at = now;
            delivery.receipts.push(Receipt {
                at: now,
                state: ReceiptState::Rejected,
                detail: Some(GROUP_STOP_REASON.to_string()),
            });
            cancelled.push((
                delivery.id.clone(),
                delivery.exchange_id.clone(),
                delivery.entry_id.clone(),
            ));
        }
        for (delivery_id, exchange_id, entry_id) in cancelled {
            self.remember_delivery(&delivery_id);
            self.set_entry_status(&exchange_id, &entry_id, DeliveryStatus::Cancelled);
        }
        let mut rejected = Vec::new();
        for action in &mut self.pending {
            if action.room_id.as_deref() == Some(id.as_str())
                && action.status == PendingStatus::AwaitingUser
            {
                action.status = PendingStatus::Rejected;
                action.resolved_at = Some(now);
                rejected.push(action.id.clone());
            }
        }
        for action_id in rejected {
            self.remember_pending(&action_id);
        }
        self.push_log(
            &id,
            Actor::User,
            LogKind::Status,
            GROUP_STOP_REASON,
            None,
            now,
        );
        Ok((self.room(&id)?, signal))
    }

    pub fn enqueue(&mut self, input: Enqueue, now: i64) -> Result<Delivery, String> {
        self.insert_delivery(input, None, now)
    }

    pub fn record_unavailable(
        &mut self,
        input: Enqueue,
        reason: String,
        now: i64,
    ) -> Result<Delivery, String> {
        if reason.trim().is_empty() {
            return Err("不可用原因不能为空".into());
        }
        self.insert_delivery(input, Some(reason), now)
    }

    pub fn claim_next(
        &mut self,
        busy: &HashSet<String>,
        token: String,
        now: i64,
    ) -> Option<Delivery> {
        let mut candidates: Vec<usize> = self
            .deliveries
            .iter()
            .enumerate()
            .filter(|(_, delivery)| self.dispatchable(delivery, busy))
            .map(|(index, _)| index)
            .collect();
        candidates.sort_by(|&left, &right| {
            let a = &self.deliveries[left];
            let b = &self.deliveries[right];
            a.created_at
                .cmp(&b.created_at)
                .then_with(|| a.id.cmp(&b.id))
        });
        let index = *candidates.first()?;
        let delivery = &mut self.deliveries[index];
        delivery.status = DeliveryStatus::Claimed;
        delivery.claim_token = Some(token);
        delivery.updated_at = now;
        delivery.receipts.push(Receipt {
            at: now,
            state: ReceiptState::Accepted,
            detail: None,
        });
        let delivery_id = delivery.id.clone();
        let exchange_id = delivery.exchange_id.clone();
        let entry_id = delivery.entry_id.clone();
        let claimed = delivery.clone();
        self.remember_delivery(&delivery_id);
        self.set_entry_status(&exchange_id, &entry_id, DeliveryStatus::Claimed);
        Some(claimed)
    }

    /// Admission boundary shared with Chat's send reservation. After this
    /// returns true, group stop will not cancel the in-flight native run.
    pub fn begin_send(&mut self, id: &str, token: &str, now: i64) -> bool {
        let Some(delivery) = self
            .deliveries
            .iter_mut()
            .find(|delivery| delivery.id == id)
        else {
            return false;
        };
        if delivery.status != DeliveryStatus::Claimed
            || delivery.claim_token.as_deref() != Some(token)
            || delivery.admitted
        {
            return false;
        }
        delivery.admitted = true;
        delivery.status = DeliveryStatus::Running;
        delivery.updated_at = now;
        let delivery_id = delivery.id.clone();
        let exchange_id = delivery.exchange_id.clone();
        let entry_id = delivery.entry_id.clone();
        self.remember_delivery(&delivery_id);
        self.set_entry_status(&exchange_id, &entry_id, DeliveryStatus::Running);
        true
    }

    /// Puts an unadmitted claim back on the queue. The native send did not start.
    pub fn requeue_unadmitted(&mut self, id: &str, token: &str, now: i64) -> bool {
        let Some(delivery) = self
            .deliveries
            .iter_mut()
            .find(|delivery| delivery.id == id)
        else {
            return false;
        };
        if delivery.admitted
            || delivery.status != DeliveryStatus::Claimed
            || delivery.claim_token.as_deref() != Some(token)
        {
            return false;
        }
        delivery.status = DeliveryStatus::Queued;
        delivery.claim_token = None;
        delivery.updated_at = now;
        let delivery_id = delivery.id.clone();
        let exchange_id = delivery.exchange_id.clone();
        let entry_id = delivery.entry_id.clone();
        self.remember_delivery(&delivery_id);
        self.set_entry_status(&exchange_id, &entry_id, DeliveryStatus::Queued);
        true
    }

    pub fn still_current(&self, id: &str, token: &str) -> bool {
        self.deliveries.iter().any(|delivery| {
            delivery.id == id
                && delivery.claim_token.as_deref() == Some(token)
                && delivery.status == DeliveryStatus::Claimed
                && !delivery.admitted
        })
    }

    pub fn finish(
        &mut self,
        id: &str,
        token: &str,
        outcome: RunFinish,
        now: i64,
    ) -> Result<FinishReport, String> {
        let existing = self
            .deliveries
            .iter()
            .find(|delivery| delivery.id == id)
            .ok_or_else(|| "没有这次投递".to_string())?
            .clone();
        if existing.claim_token.as_deref() != Some(token) {
            return Err("投递认领不一致，未写入结果".into());
        }
        if existing.status == DeliveryStatus::Interrupted {
            return Err(INTERRUPT_REASON.into());
        }
        if existing.status.is_terminal() {
            return Ok(FinishReport {
                delivery: existing,
                follow_up: None,
                already_terminal: true,
            });
        }
        if !matches!(
            existing.status,
            DeliveryStatus::Claimed | DeliveryStatus::Running
        ) {
            return Err("这次投递还不能写入运行结果".into());
        }
        let mut follow = None;
        let native = match &outcome {
            RunFinish::Succeeded { native_output, .. } => Some(native_output.clone()),
            _ => None,
        };
        let (status, error, receipt) = match &outcome {
            RunFinish::Succeeded { .. } => {
                (DeliveryStatus::Succeeded, None, ReceiptState::Completed)
            }
            RunFinish::Failed { message } => (
                DeliveryStatus::Failed,
                Some(message.clone()),
                ReceiptState::Rejected,
            ),
            RunFinish::Cancelled { message } => (
                DeliveryStatus::Cancelled,
                Some(message.clone()),
                ReceiptState::Rejected,
            ),
        };
        {
            let delivery = self
                .deliveries
                .iter_mut()
                .find(|delivery| delivery.id == id)
                .expect("delivery checked");
            delivery.status = status;
            delivery.error = error.clone();
            delivery.native_output = native.clone();
            delivery.finished_at = Some(now);
            delivery.updated_at = now;
            delivery.receipts.push(Receipt {
                at: now,
                state: receipt,
                detail: error.clone(),
            });
        }
        self.remember_delivery(id);
        self.set_entry_status(&existing.exchange_id, &existing.entry_id, status);
        if let RunFinish::Succeeded {
            native_output,
            model_enqueued,
        } = &outcome
        {
            if self.should_follow_up(&existing, *model_enqueued) {
                let from = Actor::bot(&existing.to_bot_id, &existing.to_name);
                match self.enqueue(
                    Enqueue {
                        from,
                        to_bot_id: existing.from.bot_id().unwrap_or_default().to_string(),
                        to_name: match &existing.from {
                            Actor::Bot { name, .. } => name.clone(),
                            Actor::User => "用户".into(),
                        },
                        room_id: existing.room_id.clone(),
                        kind: DeliveryKind::Reply,
                        body: native_output.clone(),
                        exchange_id: Some(existing.exchange_id.clone()),
                        parent_delivery_id: Some(existing.id.clone()),
                        sensitive: false,
                        approved: true,
                        attachment_ids: Vec::new(),
                        attachment_sources: Vec::new(),
                    },
                    now,
                ) {
                    Ok(delivery) => follow = Some(delivery),
                    Err(err) => {
                        if let Some(delivery) = self
                            .deliveries
                            .iter_mut()
                            .find(|delivery| delivery.id == id)
                        {
                            delivery.receipts.push(Receipt {
                                at: now,
                                state: ReceiptState::Completed,
                                detail: Some(format!("未继续投递：{err}")),
                            });
                        }
                        if !native_output.is_empty() {
                            self.push_entry(
                                ExchangeEntry {
                                    id: new_id("ent"),
                                    at: now,
                                    actor: Actor::bot(&existing.to_bot_id, &existing.to_name),
                                    kind: DeliveryKind::Reply,
                                    body: native_output.clone(),
                                    delivery_id: existing.id.clone(),
                                    status,
                                },
                                &existing.exchange_id,
                            );
                        }
                    }
                }
            } else if !native_output.is_empty() {
                self.push_entry(
                    ExchangeEntry {
                        id: new_id("ent"),
                        at: now,
                        actor: Actor::bot(&existing.to_bot_id, &existing.to_name),
                        kind: DeliveryKind::Reply,
                        body: native_output.clone(),
                        delivery_id: existing.id.clone(),
                        status,
                    },
                    &existing.exchange_id,
                );
                if let Some(room_id) = existing.room_id.clone() {
                    self.push_log(
                        &room_id,
                        Actor::bot(&existing.to_bot_id, &existing.to_name),
                        LogKind::Message,
                        native_output,
                        Some(&existing.id),
                        now,
                    );
                }
            }
        } else if let Some(room_id) = existing.room_id.clone() {
            let detail = error.unwrap_or_else(|| "运行没有继续".into());
            self.push_log(
                &room_id,
                Actor::bot(&existing.to_bot_id, &existing.to_name),
                LogKind::Status,
                &detail,
                Some(&existing.id),
                now,
            );
        }
        Ok(FinishReport {
            delivery: self.get_delivery(id).expect("delivery remains"),
            follow_up: follow,
            already_terminal: false,
        })
    }

    pub fn mark_unavailable(
        &mut self,
        id: &str,
        token: &str,
        reason: String,
        now: i64,
    ) -> Result<Delivery, String> {
        let delivery = self
            .deliveries
            .iter_mut()
            .find(|delivery| delivery.id == id)
            .ok_or_else(|| "没有这次投递".to_string())?;
        if delivery.claim_token.as_deref() != Some(token) {
            return Err("投递认领不一致，未写入结果".into());
        }
        if delivery.admitted || delivery.status == DeliveryStatus::Running {
            return Err("运行已经开始，不能改成不可用".into());
        }
        delivery.status = DeliveryStatus::Unavailable;
        delivery.error = Some(reason.clone());
        delivery.finished_at = Some(now);
        delivery.updated_at = now;
        delivery.receipts.push(Receipt {
            at: now,
            state: ReceiptState::Unavailable,
            detail: Some(reason),
        });
        let delivery_id = delivery.id.clone();
        let exchange_id = delivery.exchange_id.clone();
        let entry_id = delivery.entry_id.clone();
        let saved = delivery.clone();
        self.remember_delivery(&delivery_id);
        self.set_entry_status(&exchange_id, &entry_id, DeliveryStatus::Unavailable);
        Ok(saved)
    }

    pub fn resolve(
        &mut self,
        action_id: &str,
        approve: bool,
        now: i64,
    ) -> Result<PendingAction, String> {
        let index = self
            .pending
            .iter()
            .position(|action| action.id == action_id)
            .ok_or_else(|| "没有这条待确认操作".to_string())?;
        if self.pending[index].status != PendingStatus::AwaitingUser {
            return Err("这条操作已经处理过".into());
        }
        let action = self.pending[index].clone();
        if approve {
            match action.kind {
                PendingKind::AddMember => {
                    let room_id = action.room_id.clone().ok_or("成员变更缺少房间")?;
                    self.ensure_room_operable(&room_id)?;
                    if !self.is_member(&room_id, &action.bot_id) {
                        let conversation_id = member_thread_id(&action.bot_id, &room_id)?;
                        let room = self.room_mut(&room_id)?;
                        if room.members.len() >= MAX_ROOM_BOTS {
                            return Err(format!("一个房间最多 {MAX_ROOM_BOTS} 个 Bot"));
                        }
                        room.members.push(RoomMember {
                            bot_id: action.bot_id.clone(),
                            name: action.bot_name.clone(),
                            conversation_id,
                            state: MemberState::Active,
                            joined_at: now,
                        });
                        room.updated_at = now;
                    }
                }
                PendingKind::RemoveMember => {
                    let room_id = action.room_id.clone().ok_or("成员变更缺少房间")?;
                    let bot_id = action.bot_id.clone();
                    {
                        let room = self.room_mut(&room_id)?;
                        room.members.retain(|member| member.bot_id != bot_id);
                        room.updated_at = now;
                    }
                    self.cancel_queued_for_member(&room_id, &bot_id, now);
                }
                PendingKind::Handoff => {
                    if let Some(delivery_id) = &action.delivery_id {
                        self.release_handoff(delivery_id, now)?;
                    }
                }
            }
        } else if action.kind == PendingKind::Handoff {
            if let Some(delivery_id) = &action.delivery_id {
                self.cancel_unadmitted(delivery_id, "用户拒绝了交接", now)?;
            }
        }
        let action = &mut self.pending[index];
        action.status = if approve {
            PendingStatus::Approved
        } else {
            PendingStatus::Rejected
        };
        action.resolved_at = Some(now);
        let saved = action.clone();
        self.remember_pending(&saved.id);
        if let Some(room_id) = saved.room_id.clone() {
            self.push_log(
                &room_id,
                Actor::User,
                LogKind::Membership,
                if approve {
                    "用户确认了敏感操作"
                } else {
                    "用户拒绝了敏感操作"
                },
                saved.delivery_id.as_deref(),
                now,
            );
        }
        Ok(saved)
    }

    fn insert_delivery(
        &mut self,
        input: Enqueue,
        unavailable: Option<String>,
        now: i64,
    ) -> Result<Delivery, String> {
        let to_bot_id = validate_bot_id(&input.to_bot_id)?.to_string();
        let body = input.body.trim().to_string();
        if body.is_empty() {
            return Err("投递内容不能为空".into());
        }
        let attachment_ids = normalize_attachment_ids(&input.attachment_ids)?;
        let attachment_sources = input.attachment_sources.clone();
        if input.from.bot_id() == Some(to_bot_id.as_str()) {
            return Err("不能投递给自己".into());
        }
        let to_name = {
            let trimmed = input.to_name.trim();
            if trimmed.is_empty() {
                to_bot_id.clone()
            } else {
                trimmed.to_string()
            }
        };
        if let Some(room_id) = &input.room_id {
            self.ensure_member(room_id, &to_bot_id)?;
            if let Some(from_id) = input.from.bot_id() {
                self.ensure_member(room_id, from_id)?;
            }
            let room = self.room(room_id)?;
            if room.status == RoomStatus::Stopped && unavailable.is_none() {
                return Err(GROUP_STOP_REASON.into());
            }
        }
        let (exchange_id, depth) = self.prepare_exchange(&input, &to_bot_id)?;
        let room_cycle = input
            .room_id
            .as_deref()
            .and_then(|room_id| self.rooms.iter().find(|room| room.id == room_id))
            .map(|room| room.cycle)
            .unwrap_or(0);
        let sensitive = input.sensitive || input.kind == DeliveryKind::Handoff;
        let status = if unavailable.is_some() {
            DeliveryStatus::Unavailable
        } else if sensitive && !input.approved {
            DeliveryStatus::AwaitingUser
        } else {
            DeliveryStatus::Queued
        };
        let delivery_id = new_id("dlv");
        let entry_id = new_id("ent");
        let delivery = Delivery {
            id: delivery_id.clone(),
            exchange_id: exchange_id.clone(),
            entry_id: entry_id.clone(),
            room_id: input.room_id.clone(),
            room_cycle,
            parent_delivery_id: input.parent_delivery_id.clone(),
            from: input.from.clone(),
            to_bot_id: to_bot_id.clone(),
            to_name,
            to_conversation_id: self
                .rooms
                .iter()
                .find(|room| input.room_id.as_deref() == Some(room.id.as_str()))
                .and_then(|room| {
                    room.members
                        .iter()
                        .find(|member| member.bot_id == to_bot_id)
                        .map(|member| member.conversation_id.clone())
                })
                .unwrap_or_else(|| bot_conversation_id(&to_bot_id)),
            kind: input.kind,
            body: body.clone(),
            attachment_ids,
            attachment_sources,
            depth,
            status,
            claim_token: None,
            admitted: false,
            sensitive,
            native_output: None,
            error: unavailable.clone(),
            receipts: vec![Receipt {
                at: now,
                state: if status == DeliveryStatus::Unavailable {
                    ReceiptState::Unavailable
                } else {
                    ReceiptState::Accepted
                },
                detail: unavailable.clone().or_else(|| {
                    (status == DeliveryStatus::AwaitingUser).then(|| "等待用户确认".into())
                }),
            }],
            created_at: now,
            updated_at: now,
            finished_at: unavailable.as_ref().map(|_| now),
        };
        self.push_entry(
            ExchangeEntry {
                id: entry_id,
                at: now,
                actor: input.from.clone(),
                kind: input.kind,
                body: body.clone(),
                delivery_id: delivery_id.clone(),
                status,
            },
            &exchange_id,
        );
        if let Some(room_id) = &input.room_id {
            let log_kind = if input.kind == DeliveryKind::Handoff {
                LogKind::Handoff
            } else {
                LogKind::Message
            };
            self.push_log(
                room_id,
                input.from.clone(),
                log_kind,
                &body,
                Some(&delivery_id),
                now,
            );
        }
        if status == DeliveryStatus::AwaitingUser {
            let action_id = new_id("act");
            self.pending.push(PendingAction {
                id: action_id.clone(),
                room_id: input.room_id.clone(),
                kind: PendingKind::Handoff,
                bot_id: to_bot_id,
                bot_name: delivery.to_name.clone(),
                delivery_id: Some(delivery.id.clone()),
                status: PendingStatus::AwaitingUser,
                created_at: now,
                resolved_at: None,
            });
            self.remember_pending(&action_id);
        }
        self.remember_delivery(&delivery.id);
        self.deliveries.push(delivery.clone());
        Ok(delivery)
    }

    fn prepare_exchange(
        &mut self,
        input: &Enqueue,
        to_bot_id: &str,
    ) -> Result<(String, u32), String> {
        let parent = input
            .parent_delivery_id
            .as_deref()
            .map(|id| {
                self.deliveries
                    .iter()
                    .find(|delivery| delivery.id == id)
                    .cloned()
                    .ok_or_else(|| "找不到要回复的投递".to_string())
            })
            .transpose()?;
        let depth = parent
            .as_ref()
            .map(|delivery| delivery.depth.saturating_add(1))
            .unwrap_or(0);
        if depth > MAX_COORDINATION_DEPTH {
            return Err("协作轮次已到上限，未再投递".into());
        }
        let exchange_id = if let Some(parent) = &parent {
            if input
                .exchange_id
                .as_deref()
                .is_some_and(|id| id != parent.exchange_id)
            {
                return Err("回复必须留在同一次交流里".into());
            }
            parent.exchange_id.clone()
        } else if let Some(id) = input.exchange_id.clone() {
            if !self.exchanges.iter().any(|exchange| exchange.id == id) {
                return Err("没有这次交流".into());
            }
            id
        } else {
            new_id("ex")
        };
        let count = self
            .deliveries
            .iter()
            .filter(|delivery| delivery.exchange_id == exchange_id)
            .count();
        if count >= MAX_DELIVERIES_PER_EXCHANGE {
            return Err("这次交流的投递已到上限，未再投递".into());
        }
        if self.deliveries.iter().any(|delivery| {
            delivery.exchange_id == exchange_id
                && delivery.to_bot_id == to_bot_id
                && !delivery.status.is_terminal()
        }) {
            return Err("该 Bot 在这次交流里还有未完成投递".into());
        }
        if !self
            .exchanges
            .iter()
            .any(|exchange| exchange.id == exchange_id)
        {
            self.exchanges.push(Exchange {
                id: exchange_id.clone(),
                room_id: input.room_id.clone(),
                entries: Vec::new(),
            });
            self.remember_exchange(&exchange_id);
        }
        Ok((exchange_id, depth))
    }

    fn dispatchable(&self, delivery: &Delivery, busy: &HashSet<String>) -> bool {
        if delivery.status != DeliveryStatus::Queued {
            return false;
        }
        if busy.contains(&delivery.to_conversation_id) {
            return false;
        }
        if self.deliveries.iter().any(|other| {
            other.id != delivery.id
                && (other.to_conversation_id == delivery.to_conversation_id
                    || other.to_bot_id == delivery.to_bot_id)
                && matches!(
                    other.status,
                    DeliveryStatus::Claimed | DeliveryStatus::Running
                )
        }) {
            return false;
        }
        if let Some(room_id) = &delivery.room_id {
            let Some(room) = self.rooms.iter().find(|room| &room.id == room_id) else {
                return false;
            };
            if room.status != RoomStatus::Active || room.cycle != delivery.room_cycle {
                return false;
            }
        }
        true
    }

    fn should_follow_up(&self, delivery: &Delivery, model_enqueued: bool) -> bool {
        if model_enqueued
            || delivery.kind == DeliveryKind::Reply
            || delivery.kind == DeliveryKind::Broadcast
            || delivery.depth != 0
        {
            return false;
        }
        let Actor::Bot { .. } = &delivery.from else {
            return false;
        };
        if let Some(room_id) = &delivery.room_id {
            let Some(room) = self.rooms.iter().find(|room| &room.id == room_id) else {
                return false;
            };
            if room.status != RoomStatus::Active || room.cycle != delivery.room_cycle {
                return false;
            }
            if delivery
                .from
                .bot_id()
                .is_some_and(|id| !room.members.iter().any(|member| member.bot_id == id))
            {
                return false;
            }
        }
        true
    }

    fn release_handoff(&mut self, delivery_id: &str, now: i64) -> Result<(), String> {
        let delivery = self
            .deliveries
            .iter_mut()
            .find(|delivery| delivery.id == delivery_id)
            .ok_or_else(|| "没有这次交接".to_string())?;
        if delivery.status != DeliveryStatus::AwaitingUser {
            return Err("交接已经不在等待确认".into());
        }
        delivery.status = DeliveryStatus::Queued;
        delivery.updated_at = now;
        let delivery_id = delivery.id.clone();
        let exchange_id = delivery.exchange_id.clone();
        let entry_id = delivery.entry_id.clone();
        self.remember_delivery(&delivery_id);
        self.set_entry_status(&exchange_id, &entry_id, DeliveryStatus::Queued);
        Ok(())
    }

    fn cancel_unadmitted(
        &mut self,
        delivery_id: &str,
        reason: &str,
        now: i64,
    ) -> Result<(), String> {
        let delivery = self
            .deliveries
            .iter_mut()
            .find(|delivery| delivery.id == delivery_id)
            .ok_or_else(|| "没有这次投递".to_string())?;
        if delivery.admitted || delivery.status.is_terminal() {
            return Ok(());
        }
        delivery.status = DeliveryStatus::Cancelled;
        delivery.error = Some(reason.to_string());
        delivery.finished_at = Some(now);
        delivery.updated_at = now;
        delivery.receipts.push(Receipt {
            at: now,
            state: ReceiptState::Rejected,
            detail: Some(reason.to_string()),
        });
        let delivery_id = delivery.id.clone();
        let exchange_id = delivery.exchange_id.clone();
        let entry_id = delivery.entry_id.clone();
        self.remember_delivery(&delivery_id);
        self.set_entry_status(&exchange_id, &entry_id, DeliveryStatus::Cancelled);
        Ok(())
    }

    fn cancel_queued_for_member(&mut self, room_id: &str, bot_id: &str, now: i64) {
        let reason = format!("Bot「{bot_id}」已离开房间，未发送");
        let mut touched = Vec::new();
        for delivery in &mut self.deliveries {
            if delivery.room_id.as_deref() != Some(room_id) || delivery.to_bot_id != bot_id {
                continue;
            }
            if delivery.admitted
                || !matches!(
                    delivery.status,
                    DeliveryStatus::Queued | DeliveryStatus::AwaitingUser | DeliveryStatus::Claimed
                )
            {
                continue;
            }
            delivery.status = DeliveryStatus::Cancelled;
            delivery.error = Some(reason.clone());
            delivery.finished_at = Some(now);
            delivery.updated_at = now;
            touched.push((
                delivery.id.clone(),
                delivery.exchange_id.clone(),
                delivery.entry_id.clone(),
            ));
        }
        for (delivery_id, exchange_id, entry_id) in touched {
            self.remember_delivery(&delivery_id);
            self.set_entry_status(&exchange_id, &entry_id, DeliveryStatus::Cancelled);
        }
    }

    fn ensure_room_operable(&self, room_id: &str) -> Result<(), String> {
        let room = self.room(room_id)?;
        if room.status == RoomStatus::Stopped {
            return Err(GROUP_STOP_REASON.into());
        }
        Ok(())
    }

    fn ensure_member(&self, room_id: &str, bot_id: &str) -> Result<(), String> {
        if self.is_member(room_id, bot_id) {
            Ok(())
        } else {
            Err(format!("Bot「{bot_id}」不是这个房间的成员"))
        }
    }

    fn is_member(&self, room_id: &str, bot_id: &str) -> bool {
        self.rooms.iter().any(|room| {
            room.id == room_id
                && room
                    .members
                    .iter()
                    .any(|member| member.bot_id == bot_id && member.state == MemberState::Active)
        })
    }

    fn room_mut(&mut self, room_id: &str) -> Result<&mut Room, String> {
        self.rooms
            .iter_mut()
            .find(|room| room.id == room_id)
            .ok_or_else(|| "没有这个协作房间".into())
    }

    fn push_entry(&mut self, entry: ExchangeEntry, exchange_id: &str) {
        if let Some(exchange) = self
            .exchanges
            .iter_mut()
            .find(|exchange| exchange.id == exchange_id)
        {
            exchange.entries.push(entry);
            if exchange.entries.len() > MAX_ENTRIES {
                let overflow = exchange.entries.len() - MAX_ENTRIES;
                exchange.entries.drain(0..overflow);
            }
        } else {
            self.exchanges.push(Exchange {
                id: exchange_id.to_string(),
                room_id: self
                    .deliveries
                    .iter()
                    .find(|delivery| delivery.exchange_id == exchange_id)
                    .and_then(|delivery| delivery.room_id.clone()),
                entries: vec![entry],
            });
        }
        self.remember_exchange(exchange_id);
    }

    fn set_entry_status(&mut self, exchange_id: &str, entry_id: &str, status: DeliveryStatus) {
        let found = {
            let Some(exchange) = self
                .exchanges
                .iter_mut()
                .find(|exchange| exchange.id == exchange_id)
            else {
                return;
            };
            if let Some(entry) = exchange
                .entries
                .iter_mut()
                .find(|entry| entry.id == entry_id)
            {
                entry.status = status;
            }
            true
        };
        if found {
            self.remember_exchange(exchange_id);
        }
    }

    fn push_log(
        &mut self,
        room_id: &str,
        actor: Actor,
        kind: LogKind,
        body: &str,
        delivery_id: Option<&str>,
        now: i64,
    ) {
        let Some(index) = self.rooms.iter().position(|room| room.id == room_id) else {
            return;
        };
        let room = &mut self.rooms[index];
        room.log.push(RoomLogEntry {
            id: new_id("log"),
            at: now,
            actor,
            kind,
            body: body.to_string(),
            delivery_id: delivery_id.map(str::to_string),
        });
        if room.log.len() > MAX_LOG {
            let overflow = room.log.len() - MAX_LOG;
            room.log.drain(0..overflow);
        }
        room.updated_at = now;
        self.remember_room(room_id);
    }

    pub fn rename_room(&mut self, room_id: &str, title: &str, now: i64) -> Result<Room, String> {
        let title = normalize_room_title(title)?;
        let id = {
            let room = self.room_mut(room_id)?;
            room.title = title;
            room.updated_at = now;
            room.id.clone()
        };
        self.push_log(&id, Actor::User, LogKind::Status, "房间已重命名", None, now);
        self.room(&id)
    }

    /// Unaddressed room message. One delivery to each other active member, in
    /// member order, capped by [`MAX_ROOM_BOTS`]. A missing target is not an error.
    pub fn broadcast(
        &mut self,
        from: Actor,
        room_id: &str,
        body: String,
        attachment_ids: Vec<String>,
        attachment_sources: Vec<String>,
        exchange_id: Option<String>,
        parent_delivery_id: Option<String>,
        now: i64,
    ) -> Result<BroadcastReceipt, String> {
        self.ensure_room_operable(room_id)?;
        if body.trim().is_empty() {
            return Err("投递内容不能为空".into());
        }
        let targets: Vec<RoomMember> = self
            .room(room_id)?
            .members
            .into_iter()
            .filter(|member| member.state == MemberState::Active)
            .filter(|member| from.bot_id() != Some(member.bot_id.as_str()))
            .collect();
        if targets.is_empty() {
            return Err("房间里没有可投递的其他成员".into());
        }
        let mut delivery_ids = Vec::new();
        let mut skipped_bot_ids = Vec::new();
        let mut shared_exchange = exchange_id;
        for (index, member) in targets.into_iter().enumerate() {
            if index >= MAX_ROOM_BOTS {
                skipped_bot_ids.push(member.bot_id);
                continue;
            }
            let delivery = self.enqueue(
                Enqueue {
                    from: from.clone(),
                    to_bot_id: member.bot_id,
                    to_name: member.name,
                    room_id: Some(room_id.to_string()),
                    kind: DeliveryKind::Broadcast,
                    body: body.clone(),
                    exchange_id: shared_exchange.clone(),
                    parent_delivery_id: parent_delivery_id.clone(),
                    sensitive: false,
                    approved: true,
                    attachment_ids: attachment_ids.clone(),
                    attachment_sources: attachment_sources.clone(),
                },
                now,
            )?;
            shared_exchange = Some(delivery.exchange_id.clone());
            delivery_ids.push(delivery.id);
        }
        Ok(BroadcastReceipt {
            room_id: room_id.to_string(),
            delivery_ids,
            skipped_bot_ids,
            bound: u32::try_from(MAX_ROOM_BOTS).unwrap_or(u32::MAX),
        })
    }

    pub fn align_member_thread(
        &mut self,
        room_id: &str,
        bot_id: &str,
        conversation_id: &str,
        now: i64,
    ) -> Result<Room, String> {
        let changed = {
            let room = self.room_mut(room_id)?;
            let member = room
                .members
                .iter_mut()
                .find(|member| member.bot_id == bot_id && member.state == MemberState::Active)
                .ok_or_else(|| format!("Bot「{bot_id}」不是这个房间的成员"))?;
            if member.conversation_id != conversation_id {
                member.conversation_id = conversation_id.to_string();
                room.updated_at = now;
                true
            } else {
                false
            }
        };
        if changed {
            self.remember_room(room_id);
        }
        self.room(room_id)
    }
}

fn member_thread_id(bot_id: &str, room_id: &str) -> Result<String, String> {
    crate::chat::bot_profile::member_conversation_id(bot_id, room_id)
}

fn normalize_members(
    room_id: &str,
    members: &[MemberInput],
    now: i64,
) -> Result<Vec<RoomMember>, String> {
    if members.is_empty() {
        return Err("房间至少要有一个 Bot".into());
    }
    if members.len() > MAX_ROOM_BOTS {
        return Err(format!("一个房间最多 {MAX_ROOM_BOTS} 个 Bot"));
    }
    let mut seen = HashSet::new();
    let mut out = Vec::with_capacity(members.len());
    for member in members {
        let bot_id = validate_bot_id(&member.bot_id)?.to_string();
        if !seen.insert(bot_id.clone()) {
            return Err(format!("Bot「{bot_id}」重复加入房间"));
        }
        let name = member.name.trim();
        out.push(RoomMember {
            bot_id: bot_id.clone(),
            name: if name.is_empty() {
                bot_id.clone()
            } else {
                name.to_string()
            },
            conversation_id: member_thread_id(&bot_id, room_id)?,
            state: MemberState::Active,
            joined_at: now,
        });
    }
    Ok(out)
}

fn new_id(prefix: &str) -> String {
    format!("{prefix}_{}", Uuid::new_v4())
}
