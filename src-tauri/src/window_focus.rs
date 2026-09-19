//! Independent foreground-return identities for Lens and input translation.
use std::sync::atomic::{AtomicI32, Ordering};

#[derive(Default)]
pub(crate) struct FrontmostAppState {
    lens: FocusReturnSlot,
    translator: FocusReturnSlot,
}

impl FrontmostAppState {
    pub(crate) fn lens(&self) -> &FocusReturnSlot {
        &self.lens
    }
    pub(crate) fn translator(&self) -> &FocusReturnSlot {
        &self.translator
    }
}

#[derive(Default)]
pub(crate) struct FocusReturnSlot {
    pid: AtomicI32,
}

impl FocusReturnSlot {
    pub(crate) fn remember(&self, pid: i32, self_pid: i32) {
        self.pid.store(
            if pid > 0 && pid != self_pid { pid } else { 0 },
            Ordering::SeqCst,
        );
    }
    pub(crate) fn previous(&self) -> i32 {
        self.pid.load(Ordering::SeqCst)
    }
    pub(crate) fn take_previous(&self) -> i32 {
        self.pid.swap(0, Ordering::SeqCst)
    }
    pub(crate) fn forget(&self) {
        self.pid.store(0, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn slots_are_independent_and_restoration_consumes_identity_once() {
        let state = FrontmostAppState::default();
        state.lens().remember(7, 99);
        state.translator().remember(8, 99);
        assert_eq!(state.lens().previous(), 7);
        assert_eq!(state.lens().take_previous(), 7);
        assert_eq!(state.lens().take_previous(), 0);
        assert_eq!(state.translator().previous(), 8);
        state.translator().forget();
        assert_eq!(state.translator().take_previous(), 0);
        state.lens().remember(99, 99);
        assert_eq!(state.lens().previous(), 0);
    }
    #[test]
    fn concurrent_restore_has_exactly_one_consumer() {
        let slot = std::sync::Arc::new(FocusReturnSlot::default());
        slot.remember(7, 99);
        let workers: Vec<_> = (0..8)
            .map(|_| {
                let slot = slot.clone();
                std::thread::spawn(move || slot.take_previous())
            })
            .collect();
        assert_eq!(
            workers
                .into_iter()
                .map(|w| w.join().unwrap())
                .filter(|pid| *pid == 7)
                .count(),
            1
        );
    }
}
