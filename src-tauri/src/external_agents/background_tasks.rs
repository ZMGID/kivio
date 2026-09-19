//! Process-local observations of tasks owned by external CLI processes.
use std::{collections::HashMap, sync::Mutex};
/// 一条外部 CLI 后台任务（claude 的 `system/task_started` / `task_notification`）。
/// 任务本体活在 CLI 进程里，Kivio 只有观测与 `stop_task`，没有 pid、没有日志文件。
#[derive(Debug, Clone)]
pub struct ExternalBackgroundTask {
    pub task_id: String,
    pub conversation_id: String,
    /// claude 的 `task_type`：`local_bash` / `local_agent` / `remote_agent` / `local_workflow`。
    pub kind: String,
    pub description: String,
    /// `running` | `completed` | `failed` | `stopped`。
    pub status: String,
    /// 终态摘要（退出码文案 / 子代理最终回复）。
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
        task_id: &str,
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
        if let Some(entry) = map.get_mut(task_id) {
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
            }
            return;
        }
        while map.len() >= MAX_TRACKED_EXTERNAL_TASKS {
            let oldest_terminal = map
                .iter()
                .filter(|(_, t)| t.status != "running")
                .min_by_key(|(_, t)| t.started_at)
                .map(|(id, _)| id.clone());
            match oldest_terminal {
                Some(id) => map.remove(&id),
                // 全在跑（几乎不可能）：不淘汰运行中的，接受超额。
                None => break,
            };
        }
        let now = std::time::SystemTime::now();
        map.insert(
            task_id.to_string(),
            ExternalBackgroundTask {
                task_id: task_id.to_string(),
                conversation_id: conversation_id.to_string(),
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
        is_live: impl Fn(&str) -> bool,
    ) -> Vec<ExternalBackgroundTask> {
        let mut tasks = self
            .external_background_tasks
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        tasks
            .values_mut()
            .filter(|t| wanted.is_none() || Some(t.conversation_id.as_str()) == wanted)
            .map(|t| {
                if t.status == "running" && !is_live(&t.conversation_id) {
                    t.status = "stopped".into();
                    t.ended_at = Some(std::time::SystemTime::now());
                }
                t.clone()
            })
            .collect()
    }
    pub(crate) fn clear_finished(&self, wanted: Option<&str>) {
        self.external_background_tasks
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|_, t| {
                t.status == "running"
                    || (wanted.is_some() && Some(t.conversation_id.as_str()) != wanted)
            });
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn terminal_notification_preserves_metadata_and_clear_is_conversation_scoped() {
        let tasks = ExternalBackgroundTasks::default();
        tasks.upsert_external_background_task(
            "a",
            "one",
            "running",
            Some("local_agent"),
            Some("Work"),
            None,
        );
        tasks.upsert_external_background_task("b", "two", "running", None, None, None);
        tasks.upsert_external_background_task("a", "one", "completed", None, None, Some("Done"));
        let one = tasks
            .snapshot_reconciled(Some("a"), |_| true)
            .pop()
            .unwrap();
        assert_eq!(one.kind, "local_agent");
        assert_eq!(one.description, "Work");
        assert!(one.ended_at.is_some());
        tasks.clear_finished(Some("a"));
        assert!(tasks.snapshot_reconciled(Some("a"), |_| true).is_empty());
        assert_eq!(tasks.snapshot_reconciled(Some("b"), |_| true).len(), 1);
        let orphan = tasks
            .snapshot_reconciled(Some("b"), |_| false)
            .pop()
            .unwrap();
        assert_eq!(orphan.status, "stopped");
    }
}
