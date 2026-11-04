//! Agent registry (OMP `registry/agent-registry.ts`): identity, generation and
//! status `running | idle | parked | aborted`, keyed by root conversation.
//! Status is the agent's liveness, never a run outcome (that lives in progress/jobs).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatus {
    Running,
    Idle,
    Parked,
    Aborted,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRecord {
    pub id: String,
    /// Root (top-level) conversation the agent belongs to.
    pub root: String,
    /// `MAIN_AGENT_ID` or the spawning agent's id.
    pub parent_id: String,
    pub generation: u64,
    pub display_name: String,
    pub agent_type: String,
    pub status: AgentStatus,
    pub depth: u8,
    /// Synthetic conversation id the child loop runs under (usage, generations).
    pub child_conversation_id: String,
    /// Run currently executing for this agent, if any.
    pub run_id: Option<String>,
    pub session_file: PathBuf,
    pub output_file: PathBuf,
    pub last_activity: i64,
    pub activity: Option<String>,
    /// Speculative runs are not shown until the final task call adopts them.
    pub published: bool,
}

#[derive(Default)]
pub struct AgentRegistry {
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    agents: HashMap<String, HashMap<String, AgentRecord>>,
    next_generation: u64,
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// `<app_data>/subagents/<root>/`: `<id>.jsonl` transcript and `<id>.md` output.
/// `root` must be a valid conversation id, so it can never escape `subagents/`.
pub fn artifacts_dir(app: &tauri::AppHandle, root: &str) -> Result<PathBuf, String> {
    use tauri::Manager;
    crate::chat::storage::validate_conversation_id(root)?;
    let base = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("app_data_dir unavailable: {e}"))?;
    Ok(base.join("subagents").join(root))
}

impl AgentRegistry {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Register a fresh agent (status `running`) with a newly minted generation.
    pub fn register(&self, mut record: AgentRecord) -> AgentRecord {
        let mut inner = self.lock();
        inner.next_generation += 1;
        record.generation = inner.next_generation;
        record.last_activity = now_ms();
        inner
            .agents
            .entry(record.root.clone())
            .or_default()
            .insert(record.id.clone(), record.clone());
        record
    }

    pub fn get(&self, root: &str, id: &str) -> Option<AgentRecord> {
        self.lock().agents.get(root)?.get(id).cloned()
    }

    /// Exact id first, then case-insensitive with Unicode lowercase, matching the
    /// uniqueness rule `task` uses when allocating ids (OMP history/agent lookup).
    pub fn resolve(&self, root: &str, id: &str) -> Option<AgentRecord> {
        let inner = self.lock();
        let agents = inner.agents.get(root)?;
        agents.get(id).cloned().or_else(|| {
            let wanted = id.to_lowercase();
            agents
                .values()
                .find(|agent| agent.id.to_lowercase() == wanted)
                .cloned()
        })
    }

    pub fn contains(&self, root: &str, id: &str) -> bool {
        self.lock()
            .agents
            .get(root)
            .is_some_and(|agents| agents.contains_key(id))
    }

    /// Every agent of `root`, oldest registration first.
    pub fn list(&self, root: &str) -> Vec<AgentRecord> {
        let mut list: Vec<_> = self
            .lock()
            .agents
            .get(root)
            .map(|agents| agents.values().cloned().collect())
            .unwrap_or_default();
        list.sort_by_key(|agent| agent.generation);
        list
    }

    /// Every live (running/idle) agent except `id` (OMP `listVisibleTo`).
    pub fn list_visible_to(&self, root: &str, id: &str) -> Vec<AgentRecord> {
        self.list(root)
            .into_iter()
            .filter(|agent| {
                agent.published && agent.id != id && matches!(agent.status, AgentStatus::Running | AgentStatus::Idle)
            })
            .collect()
    }

    /// Apply `update` to one agent; returns the updated record.
    pub fn update(
        &self,
        root: &str,
        id: &str,
        update: impl FnOnce(&mut AgentRecord),
    ) -> Option<AgentRecord> {
        let mut inner = self.lock();
        let agent = inner.agents.get_mut(root)?.get_mut(id)?;
        update(agent);
        agent.last_activity = now_ms();
        Some(agent.clone())
    }

    pub fn set_status(&self, root: &str, id: &str, status: AgentStatus) -> Option<AgentRecord> {
        self.update(root, id, |agent| {
            agent.status = status;
            if status != AgentStatus::Running {
                agent.run_id = None;
                agent.activity = None;
            }
        })
    }

    /// `(root, id)` of the agent whose current run is `run_id`.
    pub fn find_by_run(&self, run_id: &str) -> Option<(String, String)> {
        let inner = self.lock();
        inner.agents.iter().find_map(|(root, agents)| {
            agents
                .values()
                .find(|agent| agent.run_id.as_deref() == Some(run_id))
                .map(|agent| (root.clone(), agent.id.clone()))
        })
    }

    /// `(root, id)` of the agent running under synthetic conversation `child_conversation_id`.
    pub fn find_by_child_conversation(&self, child_conversation_id: &str) -> Option<(String, String)> {
        let inner = self.lock();
        inner.agents.iter().find_map(|(root, agents)| {
            agents
                .values()
                .find(|agent| agent.child_conversation_id == child_conversation_id)
                .map(|agent| (root.clone(), agent.id.clone()))
        })
    }

    pub fn remove_root(&self, root: &str) {
        self.lock().agents.remove(root);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(root: &str, id: &str) -> AgentRecord {
        AgentRecord {
            id: id.into(),
            root: root.into(),
            parent_id: crate::chat::sub_agent::MAIN_AGENT_ID.into(),
            generation: 0,
            display_name: id.into(),
            agent_type: "task".into(),
            status: AgentStatus::Running,
            depth: 1,
            child_conversation_id: format!("subagent-{root}-{id}"),
            run_id: Some(format!("run-{id}")),
            session_file: PathBuf::new(),
            output_file: PathBuf::new(),
            last_activity: 0,
            activity: None,
            published: true,
        }
    }

    #[test]
    fn generations_are_unique_and_visibility_excludes_self_and_parked() {
        let registry = AgentRegistry::default();
        let a = registry.register(record("c", "A"));
        let b = registry.register(record("c", "B"));
        registry.register(record("c", "C"));
        assert!(b.generation > a.generation);
        registry.set_status("c", "C", AgentStatus::Parked);
        let visible: Vec<_> = registry.list_visible_to("c", "A").into_iter().map(|r| r.id).collect();
        assert_eq!(visible, vec!["B".to_string()]);
        assert_eq!(registry.find_by_run("run-B"), Some(("c".into(), "B".into())));
        registry.set_status("c", "B", AgentStatus::Idle);
        assert_eq!(registry.find_by_run("run-B"), None, "idle clears the run");
        assert_eq!(registry.resolve("c", "a").map(|r| r.id), Some("A".into()));
    }
}
