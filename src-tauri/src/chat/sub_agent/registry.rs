//! Agent registry (OMP `registry/agent-registry.ts`): identity, generation and
//! status `running | idle | parked | aborted`, keyed by root conversation.
//! Status is the agent's liveness, never a run outcome (that lives in progress/jobs).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatus {
    Running,
    Idle,
    Parked,
    Aborted,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Serialize, Deserialize)]
struct AcceptedIdentity {
    root: String,
    id: String,
}

fn acceptance_path(session_file: &Path) -> PathBuf {
    session_file.with_extension("jsonl.accepted")
}

pub(crate) fn is_accepted(session_file: &Path, root: &str, id: &str) -> Result<bool, String> {
    let path = acceptance_path(session_file);
    let metadata = match std::fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(format!("Cannot inspect subagent acceptance: {error}")),
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Ok(false);
    }
    let bytes =
        std::fs::read(path).map_err(|error| format!("Cannot read subagent acceptance: {error}"))?;
    Ok(serde_json::from_slice::<AcceptedIdentity>(&bytes)
        .is_ok_and(|accepted| accepted.root == root && accepted.id == id))
}

impl AgentRegistry {
    pub(crate) fn publish(
        &self,
        root: &str,
        id: &str,
        generation: u64,
    ) -> Result<AgentRecord, String> {
        let mut inner = self.lock();
        let record = inner
            .agents
            .get_mut(root)
            .and_then(|agents| agents.get_mut(id))
            .filter(|record| {
                record.generation == generation && record.status != AgentStatus::Aborted
            })
            .ok_or("Subagent changed before task acceptance")?;
        let accepted = serde_json::to_string(&AcceptedIdentity {
            root: root.into(),
            id: id.into(),
        })
        .map_err(|error| error.to_string())?;
        crate::chat::storage::atomic_write(
            &acceptance_path(&record.session_file),
            &accepted,
            "subagent acceptance",
        )?;
        record.published = true;
        record.last_activity = now_ms();
        Ok(record.clone())
    }

    pub(crate) fn accepted_lineage(&self, record: &AgentRecord) -> Result<bool, String> {
        let mut current = record.clone();
        let mut seen = std::collections::HashSet::new();
        loop {
            if !seen.insert(current.id.clone())
                || !current.published
                || !is_accepted(&current.session_file, &current.root, &current.id)?
            {
                return Ok(false);
            }
            if current.parent_id == super::MAIN_AGENT_ID {
                return Ok(true);
            }
            let Some(parent) = self.get(&current.root, &current.parent_id) else {
                return Ok(false);
            };
            current = parent;
        }
    }

    pub fn update_checked(
        &self,
        root: &str,
        id: &str,
        generation: u64,
        run: Option<&str>,
        update: impl FnOnce(&mut AgentRecord),
    ) -> Option<AgentRecord> {
        let mut inner = self.lock();
        let agent = inner.agents.get_mut(root)?.get_mut(id)?;
        if agent.generation != generation
            || agent.status == AgentStatus::Aborted
            || run.is_some_and(|run| agent.run_id.as_deref() != Some(run))
        {
            return None;
        }
        update(agent);
        agent.last_activity = now_ms();
        Some(agent.clone())
    }

    pub fn restore(&self, record: AgentRecord) -> AgentRecord {
        let mut inner = self.lock();
        inner.next_generation = inner.next_generation.max(record.generation);
        inner
            .agents
            .entry(record.root.clone())
            .or_default()
            .entry(record.id.clone())
            .or_insert(record)
            .clone()
    }

    pub fn is_ancestor(&self, root: &str, ancestor: &str, child: &str) -> bool {
        let mut child = child.to_string();
        let mut seen = std::collections::HashSet::new();
        while seen.insert(child.clone()) {
            let Some(record) = self.get(root, &child) else {
                return false;
            };
            if record.parent_id == ancestor {
                return true;
            }
            child = record.parent_id;
        }
        false
    }

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

    pub(crate) fn is_published(&self, root: &str, id: &str) -> bool {
        let mut id = id.to_string();
        let mut seen = std::collections::HashSet::new();
        while seen.insert(id.clone()) {
            let Some(record) = self.get(root, &id).filter(|record| record.published) else {
                return false;
            };
            if record.parent_id == super::MAIN_AGENT_ID {
                return true;
            }
            id = record.parent_id;
        }
        false
    }

    /// Every live (running/idle) agent except `id` (OMP `listVisibleTo`).
    pub fn list_visible_to(&self, root: &str, id: &str) -> Vec<AgentRecord> {
        self.list(root)
            .into_iter()
            .filter(|agent| {
                self.is_published(root, &agent.id)
                    && agent.id != id
                    && matches!(agent.status, AgentStatus::Running | AgentStatus::Idle)
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
    pub fn find_by_child_conversation(
        &self,
        child_conversation_id: &str,
    ) -> Option<(String, String)> {
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
        let visible: Vec<_> = registry
            .list_visible_to("c", "A")
            .into_iter()
            .map(|r| r.id)
            .collect();
        assert_eq!(visible, vec!["B".to_string()]);
        assert_eq!(
            registry.find_by_run("run-B"),
            Some(("c".into(), "B".into()))
        );
        registry.set_status("c", "B", AgentStatus::Idle);
        assert_eq!(registry.find_by_run("run-B"), None, "idle clears the run");
        assert_eq!(registry.resolve("c", "a").map(|r| r.id), Some("A".into()));
    }

    #[test]
    fn publication_commits_matching_acceptance_and_failure_keeps_identity_hidden() {
        let directory = tempfile::tempdir().unwrap();
        let registry = AgentRegistry::default();
        let mut a = record("root", "A");
        a.session_file = directory.path().join("A.jsonl");
        a.published = false;
        let a = registry.register(a);
        assert!(!is_accepted(&a.session_file, "root", "A").unwrap());
        assert!(!registry.accepted_lineage(&a).unwrap());
        assert!(registry.publish("root", "A", a.generation + 1).is_err());
        let accepted = registry.publish("root", "A", a.generation).unwrap();
        assert!(accepted.published && registry.accepted_lineage(&accepted).unwrap());
        assert!(!is_accepted(&a.session_file, "other", "A").unwrap());
        assert!(!is_accepted(&a.session_file, "root", "B").unwrap());
        let reopened = registry.register(accepted);
        assert!(
            registry.accepted_lineage(&reopened).unwrap(),
            "an accepted identity remains accepted after explicit generation advance"
        );
        std::fs::write(acceptance_path(&a.session_file), b"{broken").unwrap();
        assert!(!registry.accepted_lineage(&reopened).unwrap());

        let blocked = directory.path().join("blocked");
        std::fs::write(&blocked, "a file, not a directory").unwrap();
        let mut b = record("root", "B");
        b.published = false;
        b.session_file = blocked.join("B.jsonl");
        let b = registry.register(b);
        assert!(registry.publish("root", "B", b.generation).is_err());
        assert!(!registry.get("root", "B").unwrap().published);
    }

    #[test]
    fn accepted_child_stays_hidden_until_its_speculative_parent_is_accepted() {
        let directory = tempfile::tempdir().unwrap();
        let registry = AgentRegistry::default();
        let mut parent = record("root", "Parent");
        parent.published = false;
        parent.session_file = directory.path().join("Parent.jsonl");
        let parent = registry.register(parent);
        let mut child = record("root", "Parent.Child");
        child.published = false;
        child.parent_id = parent.id.clone();
        child.session_file = directory.path().join("Parent.Child.jsonl");
        let child = registry.register(child);
        registry
            .publish("root", &child.id, child.generation)
            .unwrap();
        assert!(!registry.is_published("root", &child.id));
        assert!(registry.list_visible_to("root", "Main").is_empty());
        registry
            .publish("root", &parent.id, parent.generation)
            .unwrap();
        assert!(registry.is_published("root", &child.id));
        assert_eq!(registry.list_visible_to("root", "Main").len(), 2);
    }
}
