//! Process-local observations of tasks owned by external CLI processes.
use std::{collections::HashMap, sync::Mutex};

use super::session::live::SessionIdentity;

#[derive(Debug, Clone)]
pub struct ExternalBackgroundTask {
    pub task_id: String,
    pub conversation_id: String,
    pub(crate) native_task_id: String,
    pub(crate) owner: SessionIdentity,
    pub kind: String,
    pub description: String,
    pub status: String,
    pub summary: Option<String>,
    pub started_at: std::time::SystemTime,
    pub ended_at: Option<std::time::SystemTime>,
}

#[derive(Default)]
pub(crate) struct ExternalBackgroundTasks {
    external_background_tasks: Mutex<HashMap<String, ExternalBackgroundTask>>,
}

impl ExternalBackgroundTasks {
    pub fn upsert_external_background_task(
        &self,
        conversation_id: &str,
        owner: &SessionIdentity,
        native_task_id: &str,
        status: &str,
        kind: Option<&str>,
        description: Option<&str>,
        summary: Option<&str>,
    ) {
        const MAX_TRACKED_EXTERNAL_TASKS: usize = 64;
        let terminal = status != "running";
        let mut map = self
            .external_background_tasks
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(entry) = map.values_mut().find(|entry| {
            entry.conversation_id == conversation_id
                && entry.native_task_id == native_task_id
                && entry.owner.same_instance(owner)
        }) {
            entry.status = status.to_string();
            if let Some(kind) = kind {
                entry.kind = kind.to_string();
            }
            if let Some(description) = description {
                entry.description = description.to_string();
            }
            if summary.is_some() {
                entry.summary = summary.map(str::to_string);
            }
            if terminal && entry.ended_at.is_none() {
                entry.ended_at = Some(std::time::SystemTime::now());
            } else if !terminal {
                entry.ended_at = None;
            }
            return;
        }
        while map.len() >= MAX_TRACKED_EXTERNAL_TASKS {
            let oldest_terminal = map
                .iter()
                .filter(|(_, task)| task.status != "running")
                .min_by_key(|(_, task)| task.started_at)
                .map(|(id, _)| id.clone());
            match oldest_terminal {
                Some(id) => {
                    map.remove(&id);
                }
                None => break,
            }
        }
        let now = std::time::SystemTime::now();
        let task_id = format!("ext-task-{}", uuid::Uuid::new_v4());
        map.insert(
            task_id.clone(),
            ExternalBackgroundTask {
                task_id,
                conversation_id: conversation_id.to_string(),
                native_task_id: native_task_id.to_string(),
                owner: owner.clone(),
                kind: kind.unwrap_or("local_bash").to_string(),
                description: description.unwrap_or_default().to_string(),
                status: status.to_string(),
                summary: summary.map(str::to_string),
                started_at: now,
                ended_at: terminal.then_some(now),
            },
        );
    }

    pub(crate) fn snapshot_reconciled(
        &self,
        wanted: Option<&str>,
        is_live: impl Fn(&str, &SessionIdentity) -> bool,
    ) -> Vec<ExternalBackgroundTask> {
        let snapshot: Vec<_> = self
            .external_background_tasks
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .filter(|task| wanted.is_none() || Some(task.conversation_id.as_str()) == wanted)
            .cloned()
            .collect();
        snapshot
            .into_iter()
            .map(|mut task| {
                if task.status == "running" && !is_live(&task.conversation_id, &task.owner) {
                    self.mark_stopped(&task.conversation_id, &task.task_id);
                    if let Some(current) = self.lookup_stop(&task.conversation_id, &task.task_id) {
                        task = current;
                    }
                }
                task
            })
            .collect()
    }

    pub(crate) fn lookup_stop(
        &self,
        conversation_id: &str,
        task_id: &str,
    ) -> Option<ExternalBackgroundTask> {
        self.external_background_tasks
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(task_id)
            .filter(|task| task.conversation_id == conversation_id)
            .cloned()
    }

    pub(crate) fn mark_stopped(&self, conversation_id: &str, task_id: &str) {
        if let Some(task) = self
            .external_background_tasks
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get_mut(task_id)
            .filter(|task| task.conversation_id == conversation_id && task.status == "running")
        {
            task.status = "stopped".into();
            task.ended_at = Some(std::time::SystemTime::now());
        }
    }

    pub(crate) fn clear_finished(&self, wanted: Option<&str>) {
        self.external_background_tasks
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|_, task| {
                task.status == "running"
                    || (wanted.is_some() && Some(task.conversation_id.as_str()) != wanted)
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::external_agents::session::live::{SessionCommand, SessionControl};
    use tokio::sync::mpsc;

    fn owner() -> (
        SessionControl,
        SessionIdentity,
        mpsc::Receiver<SessionCommand>,
    ) {
        let (tx, rx) = mpsc::channel(4);
        let control = SessionControl::spawn(tx, std::future::pending());
        let identity = SessionIdentity::from_control(&control);
        (control, identity, rx)
    }

    #[tokio::test]
    async fn native_task_ids_are_isolated_by_conversation_and_process() {
        let tasks = ExternalBackgroundTasks::default();
        let (_a, a, _rx_a) = owner();
        let (_b, b, _rx_b) = owner();
        tasks.upsert_external_background_task(
            "a",
            &a,
            "one",
            "running",
            Some("local_agent"),
            Some("A"),
            None,
        );
        tasks.upsert_external_background_task("b", &b, "one", "running", None, Some("B"), None);
        let first = tasks
            .snapshot_reconciled(Some("a"), |_, _| true)
            .pop()
            .unwrap();
        assert_eq!(first.description, "A");
        let second = tasks
            .snapshot_reconciled(Some("b"), |_, _| true)
            .pop()
            .unwrap();
        assert_eq!(second.description, "B");
        assert_ne!(first.task_id, second.task_id);
        tasks.mark_stopped("a", &first.task_id);
        assert_eq!(
            tasks.lookup_stop("a", &first.task_id).unwrap().status,
            "stopped"
        );
        tasks.upsert_external_background_task("a", &a, "one", "running", None, None, None);
        let still_running = tasks.lookup_stop("a", &first.task_id).unwrap();
        assert_eq!(still_running.status, "running");
        assert!(still_running.ended_at.is_none());
        tasks.clear_finished(Some("a"));
        assert!(tasks.lookup_stop("a", &first.task_id).is_some());
        tasks.upsert_external_background_task(
            "a",
            &a,
            "one",
            "completed",
            None,
            None,
            Some("Done"),
        );
        let completed = tasks.lookup_stop("a", &first.task_id).unwrap();
        assert_eq!(completed.kind, "local_agent");
        assert_eq!(completed.status, "completed");
        assert_eq!(completed.summary.as_deref(), Some("Done"));
        assert!(completed.ended_at.is_some());
        tasks.clear_finished(Some("a"));
        assert!(tasks.snapshot_reconciled(Some("a"), |_, _| true).is_empty());
        assert_eq!(
            tasks.snapshot_reconciled(Some("b"), |_, _| false)[0].status,
            "stopped"
        );
    }

    #[tokio::test]
    async fn stale_task_stop_targets_only_its_original_actor() {
        let tasks = ExternalBackgroundTasks::default();
        let (old_control, old, mut old_rx) = owner();
        let (_new_control, new, mut new_rx) = owner();
        tasks.upsert_external_background_task(
            "a",
            &old,
            "same",
            "running",
            None,
            Some("old"),
            None,
        );
        let old_task = tasks
            .snapshot_reconciled(Some("a"), |_, _| true)
            .pop()
            .unwrap();
        tasks.upsert_external_background_task(
            "a",
            &new,
            "same",
            "running",
            None,
            Some("new"),
            None,
        );
        let rows = tasks.snapshot_reconciled(Some("a"), |_, _| true);
        assert_eq!(rows.len(), 2);
        assert!(tasks.lookup_stop("b", &old_task.task_id).is_none());
        let target = tasks.lookup_stop("a", &old_task.task_id).unwrap();
        target
            .owner
            .sender()
            .unwrap()
            .send(SessionCommand::StopTask {
                task_id: target.native_task_id,
            })
            .await
            .unwrap();
        assert!(
            matches!(old_rx.recv().await, Some(SessionCommand::StopTask { task_id }) if task_id == "same")
        );
        assert!(new_rx.try_recv().is_err());
        drop(old_rx);
        drop(old_control);
        assert!(target
            .owner
            .sender()
            .is_none_or(|sender| sender.is_closed()));
        let rows = tasks.snapshot_reconciled(Some("a"), |_, identity| identity.same_instance(&new));
        assert_eq!(
            rows.iter()
                .find(|row| row.task_id == old_task.task_id)
                .unwrap()
                .status,
            "stopped"
        );
        assert_eq!(
            rows.iter()
                .find(|row| row.task_id != old_task.task_id)
                .unwrap()
                .status,
            "running"
        );
    }

    #[tokio::test]
    async fn same_actor_keeps_one_observation_across_turns_and_snapshot_does_not_hold_lock() {
        let tasks = ExternalBackgroundTasks::default();
        let (control, original, _rx) = owner();
        tasks.upsert_external_background_task(
            "a",
            &original,
            "one",
            "running",
            None,
            Some("work"),
            None,
        );
        let id = tasks.snapshot_reconciled(Some("a"), |_, _| true)[0]
            .task_id
            .clone();
        let reused = SessionIdentity::from_control(&control);
        tasks.upsert_external_background_task(
            "a",
            &reused,
            "one",
            "completed",
            None,
            None,
            Some("done"),
        );
        let rows = tasks.snapshot_reconciled(Some("a"), |conversation, _| {
            tasks.clear_finished(Some(conversation));
            true
        });
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].task_id, id);
        assert_eq!(rows[0].status, "completed");
        tasks.upsert_external_background_task("a", &reused, "two", "running", None, None, None);
        let rows = tasks.snapshot_reconciled(Some("a"), |conversation, _| {
            tasks.clear_finished(Some(conversation));
            false
        });
        assert!(rows.iter().all(|row| row.status != "running"));
    }
}
