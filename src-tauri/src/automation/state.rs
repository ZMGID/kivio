use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::{Mutex, MutexGuard};

const DEFAULT_MAX_CONCURRENT_RUNS: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BeginRunError {
    AlreadyRunning,
    AtCapacity { max: usize },
}

impl fmt::Display for BeginRunError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyRunning => formatter.write_str("automation is already running"),
            Self::AtCapacity { max } => {
                write!(
                    formatter,
                    "too many automations running concurrently (max {max})"
                )
            }
        }
    }
}

#[derive(Debug, Default)]
struct RunIndexes {
    active: HashMap<String, String>,
    cancelled: HashSet<String>,
}

/// Owns the complete in-memory lifecycle of automation runs.
///
/// Keeping both indexes behind one lock makes begin/cancel/finish transitions atomic and prevents
/// callers from observing or mutating only half of the lifecycle state.
#[derive(Debug)]
pub(crate) struct AutomationRunState {
    max_concurrent: usize,
    indexes: Mutex<RunIndexes>,
}

impl Default for AutomationRunState {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_CONCURRENT_RUNS)
    }
}

impl AutomationRunState {
    #[cfg(test)]
    fn with_capacity(max_concurrent: usize) -> Self {
        Self::new(max_concurrent)
    }

    fn new(max_concurrent: usize) -> Self {
        Self {
            max_concurrent: max_concurrent.max(1),
            indexes: Mutex::new(RunIndexes::default()),
        }
    }

    fn indexes(&self) -> MutexGuard<'_, RunIndexes> {
        self.indexes
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }

    pub(crate) fn begin(&self, automation_id: &str, run_id: &str) -> Result<(), BeginRunError> {
        let mut indexes = self.indexes();
        if indexes.active.contains_key(automation_id) {
            return Err(BeginRunError::AlreadyRunning);
        }
        if indexes.active.len() >= self.max_concurrent {
            return Err(BeginRunError::AtCapacity {
                max: self.max_concurrent,
            });
        }
        indexes
            .active
            .insert(automation_id.to_string(), run_id.to_string());
        Ok(())
    }

    pub(crate) fn active_run(&self, automation_id: &str) -> Option<String> {
        self.indexes().active.get(automation_id).cloned()
    }

    pub(crate) fn is_active(&self, automation_id: &str, run_id: &str) -> bool {
        self.indexes()
            .active
            .get(automation_id)
            .is_some_and(|active_run_id| active_run_id == run_id)
    }

    pub(crate) fn active_automation_ids(&self) -> Vec<String> {
        self.indexes().active.keys().cloned().collect()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.indexes().active.is_empty()
    }

    pub(crate) fn mark_cancelled(&self, automation_id: &str) -> Option<String> {
        let mut indexes = self.indexes();
        let run_id = indexes.active.get(automation_id)?.clone();
        indexes.cancelled.insert(run_id.clone());
        Some(run_id)
    }

    pub(crate) fn is_cancelled(&self, run_id: &str) -> bool {
        self.indexes().cancelled.contains(run_id)
    }

    pub(crate) fn finish(&self, automation_id: &str, run_id: &str) {
        let mut indexes = self.indexes();
        if indexes
            .active
            .get(automation_id)
            .is_some_and(|active_run_id| active_run_id == run_id)
        {
            indexes.active.remove(automation_id);
        }
        indexes.cancelled.remove(run_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn begin_rejects_duplicates_and_capacity_atomically() {
        let runs = AutomationRunState::with_capacity(2);
        assert!(runs.begin("a", "run-a").is_ok());
        assert_eq!(
            runs.begin("a", "run-a-duplicate"),
            Err(BeginRunError::AlreadyRunning)
        );
        assert!(runs.begin("b", "run-b").is_ok());
        assert_eq!(
            runs.begin("c", "run-c"),
            Err(BeginRunError::AtCapacity { max: 2 })
        );
        assert_eq!(runs.active_run("a").as_deref(), Some("run-a"));
        assert!(runs.is_active("b", "run-b"));
        assert_eq!(runs.active_automation_ids().len(), 2);
    }

    #[test]
    fn cancel_marks_active_run_and_finish_cleans_both_indexes() {
        let runs = AutomationRunState::with_capacity(1);
        runs.begin("a", "run-a").unwrap();

        assert_eq!(runs.mark_cancelled("a").as_deref(), Some("run-a"));
        assert!(runs.is_cancelled("run-a"));

        runs.finish("a", "run-a");
        assert!(runs.active_run("a").is_none());
        assert!(!runs.is_cancelled("run-a"));
        assert!(runs.is_empty());
    }

    #[test]
    fn stale_finish_does_not_remove_a_different_active_run() {
        let runs = AutomationRunState::with_capacity(1);
        runs.begin("a", "current").unwrap();

        runs.finish("a", "stale");

        assert_eq!(runs.active_run("a").as_deref(), Some("current"));
    }
}
