// Adapted from Zeron shell.rs::NavHistory at c04487045e5f934818ab21bf5b7b7fbed16ef3d6.
// MIT; see ../../vendor/zeron-ui-LICENSE. Entries are Kivio conversation IDs;
// settings do not replace the selected conversation or enter this history.
#[derive(Debug)]
pub(super) struct NavHistory {
    entries: Vec<usize>,
    index: usize,
}

impl NavHistory {
    pub(super) fn new(initial: usize) -> Self {
        Self {
            entries: vec![initial],
            index: 0,
        }
    }

    pub(super) fn current(&self) -> &usize {
        &self.entries[self.index]
    }

    /// Record a route change. Re-navigating to the current route is a no-op
    /// (selecting the already-selected chat never happened as a navigation);
    /// otherwise any forward branch is truncated and the entry appended.
    pub(super) fn push(&mut self, entry: usize) {
        if *self.current() == entry {
            return;
        }
        self.entries.truncate(self.index + 1);
        self.entries.push(entry);
        self.index += 1;
    }

    pub(super) fn can_back(&self) -> bool {
        self.index > 0
    }

    /// Memory history keeps every entry, so "behind the last entry" is exactly
    /// "can go forward" (zeron window-controls.tsx).
    pub(super) fn can_forward(&self) -> bool {
        self.index + 1 < self.entries.len()
    }

    pub(super) fn back(&mut self) -> Option<usize> {
        if !self.can_back() {
            return None;
        }
        self.index -= 1;
        Some(*self.current())
    }

    pub(super) fn forward(&mut self) -> Option<usize> {
        if !self.can_forward() {
            return None;
        }
        self.index += 1;
        Some(*self.current())
    }
}
