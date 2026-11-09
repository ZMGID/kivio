use super::injection::*;
use super::{error::AgentFailure, host::SessionInput};
use std::sync::{Arc, Mutex, Weak};
use tokio::sync::{oneshot, watch};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WakePolicy {
    AtBoundary,
    WhenIdle,
}

pub struct InjectionReceipt(oneshot::Receiver<InjectionOutcome>);
impl InjectionReceipt {
    pub async fn wait(self) -> InjectionOutcome {
        self.0
            .await
            .unwrap_or(InjectionOutcome::Dropped(DropReason::Released))
    }
}
struct Receipt(Mutex<Option<oneshot::Sender<InjectionOutcome>>>);
impl Receipt {
    fn settle(&self, outcome: InjectionOutcome) {
        let sender = self.0.lock().unwrap().take();
        if let Some(sender) = sender {
            let _ = sender.send(outcome);
        }
    }
    fn pending(&self) -> bool {
        self.0.lock().unwrap().is_some()
    }
}

pub trait InjectionKind: Send + Sync + 'static {
    type Item: Send + 'static;
    fn is_stale(&self, _item: &Self::Item, _context: InjectionContext<'_>) -> bool {
        false
    }
    /// Runs outside queue locks. Perform producer I/O before enqueueing.
    fn build(
        &self,
        items: &[Self::Item],
        context: InjectionContext<'_>,
    ) -> Result<Vec<SessionInput>, InjectionError>;
}

trait QueuedKind: Send + Sync {
    fn drain(self: Arc<Self>) -> Option<InjectionBatch>;
    fn pending(&self, waking_only: bool) -> bool;
    fn open(&self) -> bool;
    fn invalidate(&self, close: bool);
}
struct QueueState {
    kinds: Mutex<Vec<Arc<dyn QueuedKind>>>,
    changes: watch::Sender<u64>,
}
impl QueueState {
    fn changed(&self) {
        self.changes
            .send_modify(|revision| *revision = revision.wrapping_add(1));
    }
}
impl Drop for QueueState {
    fn drop(&mut self) {
        for kind in self.kinds.get_mut().unwrap() {
            kind.invalidate(true);
        }
    }
}
#[derive(Clone)]
pub struct InjectionQueue(Arc<QueueState>);
impl Default for InjectionQueue {
    fn default() -> Self {
        Self::new()
    }
}
impl InjectionQueue {
    pub fn new() -> Self {
        let (changes, _) = watch::channel(0);
        Self(Arc::new(QueueState {
            kinds: Mutex::new(Vec::new()),
            changes,
        }))
    }
    pub fn register<K: InjectionKind>(
        &self,
        kind: K,
    ) -> Result<InjectionSender<K::Item>, InjectionError> {
        let state = Arc::new(KindState {
            data: Mutex::new(KindData {
                epoch: 0,
                open: true,
                queued: Vec::new(),
                reserved: Vec::new(),
            }),
            kind: Arc::new(kind),
            queue: Arc::downgrade(&self.0),
        });
        self.0.kinds.lock().unwrap().push(state.clone());
        Ok(InjectionSender { state })
    }
    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.0.changes.subscribe()
    }
    pub fn has_pending(&self) -> bool {
        self.0
            .kinds
            .lock()
            .unwrap()
            .iter()
            .any(|kind| kind.pending(false))
    }
    pub fn has_waking_input(&self) -> bool {
        self.0
            .kinds
            .lock()
            .unwrap()
            .iter()
            .any(|kind| kind.pending(true))
    }
}
impl InjectionSource for InjectionQueue {
    fn collect<'a>(
        &'a mut self,
        _context: InjectionContext<'a>,
        admission: &'a mut InjectionAdmission,
    ) -> InjectionFuture<'a, Result<(), InjectionError>> {
        Box::pin(async move {
            let kinds = self.0.kinds.lock().unwrap().clone();
            for kind in kinds {
                if let Some(batch) = kind.drain() {
                    admission.pending(batch);
                }
            }
            Ok(())
        })
    }
}

struct Entry<T> {
    item: T,
    receipt: Arc<Receipt>,
    wake: WakePolicy,
}
struct KindData<T> {
    epoch: u64,
    open: bool,
    queued: Vec<Entry<T>>,
    reserved: Vec<(Weak<Receipt>, WakePolicy)>,
}
struct KindState<T> {
    data: Mutex<KindData<T>>,
    kind: Arc<dyn InjectionKind<Item = T>>,
    queue: Weak<QueueState>,
}
pub struct InjectionSender<T> {
    state: Arc<KindState<T>>,
}
impl<T> Clone for InjectionSender<T> {
    fn clone(&self) -> Self {
        Self {
            state: self.state.clone(),
        }
    }
}
impl<T: Send + 'static> InjectionSender<T> {
    pub fn enqueue(&self, item: T, wake: WakePolicy) -> Result<InjectionReceipt, InjectionError> {
        let (sender, receiver) = oneshot::channel();
        {
            let mut data = self.state.data.lock().unwrap();
            if !data.open {
                return Err(InjectionError::Unregistered);
            }
            data.queued.push(Entry {
                item,
                receipt: Arc::new(Receipt(Mutex::new(Some(sender)))),
                wake,
            });
        }
        if let Some(queue) = self.state.queue.upgrade() {
            queue.changed();
        }
        Ok(InjectionReceipt(receiver))
    }
    pub fn clear(&self) {
        self.invalidate(false);
    }
    pub fn unregister(&self) {
        self.invalidate(true);
    }
    fn invalidate(&self, close: bool) {
        self.state.invalidate(close);
        if let Some(queue) = self.state.queue.upgrade() {
            if close {
                queue.kinds.lock().unwrap().retain(|kind| kind.open());
            }
            queue.changed();
        }
    }
}
impl<T: Send + 'static> QueuedKind for KindState<T> {
    fn drain(self: Arc<Self>) -> Option<InjectionBatch> {
        let (entries, epoch) = {
            let mut data = self.data.lock().unwrap();
            if data.queued.is_empty() {
                return None;
            }
            let entries = std::mem::take(&mut data.queued);
            data.reserved
                .retain(|(receipt, _)| receipt.upgrade().is_some_and(|receipt| receipt.pending()));
            data.reserved.extend(
                entries
                    .iter()
                    .map(|entry| (Arc::downgrade(&entry.receipt), entry.wake)),
            );
            (entries, data.epoch)
        };
        let mut batch = InjectionBatch::default();
        batch.lazy = Some(Box::new(KindBatch {
            state: self,
            entries,
            epoch,
        }));
        Some(batch)
    }
    fn pending(&self, waking_only: bool) -> bool {
        let data = self.data.lock().unwrap();
        data.queued
            .iter()
            .any(|entry| !waking_only || entry.wake == WakePolicy::WhenIdle)
            || data.reserved.iter().any(|(receipt, wake)| {
                (!waking_only || *wake == WakePolicy::WhenIdle)
                    && receipt.upgrade().is_some_and(|receipt| receipt.pending())
            })
    }
    fn open(&self) -> bool {
        self.data.lock().unwrap().open
    }
    fn invalidate(&self, close: bool) {
        let (queued, senders) = {
            let mut data = self.data.lock().unwrap();
            data.epoch = data.epoch.wrapping_add(1);
            if close {
                data.open = false;
            }
            let queued = std::mem::take(&mut data.queued);
            let senders: Vec<_> = queued
                .iter()
                .map(|entry| entry.receipt.clone())
                .chain(
                    data.reserved
                        .drain(..)
                        .filter_map(|(receipt, _)| receipt.upgrade()),
                )
                .filter_map(|receipt| receipt.0.lock().unwrap().take())
                .collect();
            (queued, senders)
        };
        let outcome = InjectionOutcome::Dropped(if close {
            DropReason::Unregistered
        } else {
            DropReason::Cleared
        });
        for sender in senders {
            let _ = sender.send(outcome.clone());
        }
        drop(queued);
    }
}
struct KindBatch<T> {
    state: Arc<KindState<T>>,
    entries: Vec<Entry<T>>,
    epoch: u64,
}
impl<T: Send + 'static> PendingInjection for KindBatch<T> {
    fn build(
        &mut self,
        context: InjectionContext<'_>,
    ) -> Result<Vec<SessionInput>, InjectionError> {
        self.entries.retain(|entry| {
            if !entry.receipt.pending() {
                return false;
            }
            if self.state.kind.is_stale(&entry.item, context) {
                entry
                    .receipt
                    .settle(InjectionOutcome::Dropped(DropReason::Stale));
                false
            } else {
                true
            }
        });
        if self.entries.is_empty() {
            return Ok(Vec::new());
        }
        // Move values together so the public builder receives one typed slice per kind.
        let entries = std::mem::take(&mut self.entries);
        let (items, receipts): (Vec<_>, Vec<_>) = entries
            .into_iter()
            .map(|entry| (entry.item, (entry.receipt, entry.wake)))
            .unzip();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.state.kind.build(&items, context)
        }))
        .unwrap_or(Err(InjectionError::Panicked));
        self.entries = items
            .into_iter()
            .zip(receipts)
            .map(|(item, (receipt, wake))| Entry {
                item,
                receipt,
                wake,
            })
            .collect();
        result
    }
    fn apply(
        &mut self,
        inputs: Vec<SessionInput>,
        append: &mut dyn FnMut(Vec<SessionInput>) -> Result<(), AgentFailure>,
    ) -> Result<bool, AgentFailure> {
        let data = self.state.data.lock().unwrap();
        if !data.open || data.epoch != self.epoch || inputs.is_empty() {
            return Ok(false);
        }
        // Clear and append linearize here; no producer callback runs under this lock.
        append(inputs)?;
        let senders: Vec<_> = self
            .entries
            .iter()
            .filter_map(|entry| entry.receipt.0.lock().unwrap().take())
            .collect();
        drop(data);
        for sender in senders {
            let _ = sender.send(InjectionOutcome::Applied);
        }
        Ok(true)
    }
    fn settle(&mut self, outcome: InjectionOutcome) {
        for entry in &self.entries {
            entry.receipt.settle(outcome.clone());
        }
        if let Some(queue) = self.state.queue.upgrade() {
            queue.changed();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        future::Future,
        task::{Context, Wake, Waker},
    };

    struct Kind;
    impl InjectionKind for Kind {
        type Item = ();
        fn build(
            &self,
            _: &[()],
            _: InjectionContext<'_>,
        ) -> Result<Vec<SessionInput>, InjectionError> {
            unreachable!()
        }
    }

    struct ApplyOnWake(Mutex<Option<InjectionBatch>>);
    impl Wake for ApplyOnWake {
        fn wake(self: Arc<Self>) {
            let batch = self.0.lock().unwrap().take().unwrap();
            assert!(!batch
                .apply(
                    vec![SessionInput::append(super::super::RuntimeMessage::text(
                        crate::model::ModelRole::User,
                        "revoked",
                    ))],
                    &mut |_| panic!("invalidated input was appended"),
                )
                .unwrap());
        }
    }

    #[tokio::test]
    async fn invalidation_settles_reserved_receipts_before_waking_other_receipts() {
        for close in [false, true] {
            let queue = InjectionQueue::new();
            let sender = queue.register(Kind).unwrap();
            let reserved = sender.enqueue((), WakePolicy::AtBoundary).unwrap();
            let batch = sender.state.clone().drain().unwrap();
            let queued = sender.enqueue((), WakePolicy::AtBoundary).unwrap();
            let mut waiting = Box::pin(queued.wait());
            let waker = Waker::from(Arc::new(ApplyOnWake(Mutex::new(Some(batch)))));
            assert!(waiting
                .as_mut()
                .poll(&mut Context::from_waker(&waker))
                .is_pending());
            if close {
                sender.unregister();
            } else {
                sender.clear();
            }
            let expected = InjectionOutcome::Dropped(if close {
                DropReason::Unregistered
            } else {
                DropReason::Cleared
            });
            assert_eq!(reserved.wait().await, expected);
            assert_eq!(waiting.await, expected);
            assert!(!queue.has_pending());
        }
    }
}
