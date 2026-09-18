use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex};

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use super::manager::McpSession;
use super::types::McpTool;

pub(crate) type McpSessionHandle = Arc<Mutex<McpSession>>;

pub(crate) struct SessionLookup {
    pub(crate) session: McpSessionHandle,
    pub(crate) inserted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct McpToolSnapshot {
    config_fingerprint: String,
    tools: Vec<McpTool>,
}

/// Owns MCP transport sessions and last-known tool schemas.
///
/// The pool lock is deliberately hidden: callers receive cloned `Arc`s or owned snapshots, so the
/// outer lock cannot accidentally be held across a handshake or RPC await.
pub(crate) struct McpRuntimeState {
    sessions: Mutex<HashMap<String, McpSessionHandle>>,
    tool_snapshots: StdMutex<HashMap<String, McpToolSnapshot>>,
    tool_snapshot_path: PathBuf,
}

impl McpRuntimeState {
    pub(crate) fn load(usage_dir: &Path) -> Self {
        let tool_snapshot_path = usage_dir.join("mcp-tool-snapshots.json");
        let tool_snapshots = load_tool_snapshots(&tool_snapshot_path);
        Self {
            sessions: Mutex::new(HashMap::new()),
            tool_snapshots: StdMutex::new(tool_snapshots),
            tool_snapshot_path,
        }
    }

    pub(crate) async fn get_or_insert_session<F>(&self, server_id: &str, create: F) -> SessionLookup
    where
        F: FnOnce() -> McpSessionHandle,
    {
        use std::collections::hash_map::Entry;

        let mut sessions = self.sessions.lock().await;
        match sessions.entry(server_id.to_string()) {
            Entry::Occupied(entry) => SessionLookup {
                session: entry.get().clone(),
                inserted: false,
            },
            Entry::Vacant(entry) => {
                let candidate = create();
                entry.insert(candidate.clone());
                SessionLookup {
                    session: candidate,
                    inserted: true,
                }
            }
        }
    }

    pub(crate) async fn session(&self, server_id: &str) -> Option<McpSessionHandle> {
        self.sessions.lock().await.get(server_id).cloned()
    }

    pub(crate) async fn session_entries(&self) -> Vec<(String, McpSessionHandle)> {
        self.sessions
            .lock()
            .await
            .iter()
            .map(|(id, session)| (id.clone(), session.clone()))
            .collect()
    }

    pub(crate) async fn remove_session(&self, server_id: &str) -> Option<McpSessionHandle> {
        self.sessions.lock().await.remove(server_id)
    }

    pub(crate) async fn remove_sessions(
        &self,
        server_ids: &[String],
    ) -> Vec<(String, McpSessionHandle)> {
        let mut sessions = self.sessions.lock().await;
        server_ids
            .iter()
            .filter_map(|id| sessions.remove(id).map(|session| (id.clone(), session)))
            .collect()
    }

    pub(crate) async fn drain_sessions(&self) -> Vec<(String, McpSessionHandle)> {
        self.sessions.lock().await.drain().collect()
    }

    pub(crate) fn try_session_values(&self) -> Option<Vec<McpSessionHandle>> {
        self.sessions
            .try_lock()
            .ok()
            .map(|sessions| sessions.values().cloned().collect())
    }

    #[cfg(test)]
    pub(crate) async fn sessions_empty(&self) -> bool {
        self.sessions.lock().await.is_empty()
    }

    #[cfg(test)]
    pub(crate) async fn session_count(&self) -> usize {
        self.sessions.lock().await.len()
    }

    #[cfg(test)]
    pub(crate) async fn has_session(&self, server_id: &str) -> bool {
        self.sessions.lock().await.contains_key(server_id)
    }

    pub(crate) fn tool_snapshot(
        &self,
        server_id: &str,
        config_fingerprint: &str,
    ) -> Option<Vec<McpTool>> {
        self.tool_snapshots
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(server_id)
            .filter(|snapshot| snapshot.config_fingerprint == config_fingerprint)
            .map(|snapshot| snapshot.tools.clone())
            .filter(|tools| !tools.is_empty())
    }

    pub(crate) fn set_tool_snapshot(
        &self,
        server_id: String,
        config_fingerprint: String,
        tools: Vec<McpTool>,
    ) {
        if tools.is_empty() {
            return;
        }
        let mut snapshots = self
            .tool_snapshots
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        snapshots.insert(
            server_id,
            McpToolSnapshot {
                config_fingerprint,
                tools,
            },
        );
        let content = match serde_json::to_string_pretty(&*snapshots) {
            Ok(content) => content,
            Err(error) => {
                eprintln!("Failed to serialize MCP tool snapshots: {error}");
                return;
            }
        };
        if let Err(error) = crate::chat::storage::atomic_write(
            &self.tool_snapshot_path,
            &content,
            "MCP tool snapshots",
        ) {
            eprintln!("Failed to persist MCP tool snapshots: {error}");
        }
    }
}

fn load_tool_snapshots(path: &Path) -> HashMap<String, McpToolSnapshot> {
    let Ok(content) = std::fs::read_to_string(path) else {
        return HashMap::new();
    };
    match serde_json::from_str(&content) {
        Ok(snapshots) => snapshots,
        Err(error) => {
            eprintln!(
                "Failed to load MCP tool snapshots from {}: {error}",
                path.display()
            );
            HashMap::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tokio::sync::Mutex;

    use super::*;
    use crate::mcp::manager::McpSession;
    use crate::mcp::types::McpTool;

    fn sample_tool(name: &str) -> McpTool {
        McpTool {
            name: name.to_string(),
            description: format!("{name} tool"),
            input_schema: serde_json::json!({ "type": "object" }),
            output_schema: None,
            annotations: None,
        }
    }

    #[tokio::test]
    async fn session_pool_hits_existing_entry_and_removes_it() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = McpRuntimeState::load(dir.path());
        let first = Arc::new(Mutex::new(McpSession::placeholder("first".into())));
        let second = Arc::new(Mutex::new(McpSession::placeholder("second".into())));

        let inserted = runtime.get_or_insert_session("srv", || first.clone()).await;
        assert!(inserted.inserted);
        assert!(Arc::ptr_eq(&inserted.session, &first));

        let hit = runtime.get_or_insert_session("srv", || second).await;
        assert!(!hit.inserted);
        assert!(Arc::ptr_eq(&hit.session, &first));
        assert_eq!(runtime.session_count().await, 1);

        let removed = runtime.remove_session("srv").await.expect("removed");
        assert!(Arc::ptr_eq(&removed, &first));
        assert!(runtime.sessions_empty().await);
    }

    #[test]
    fn snapshot_replacement_and_reload_preserve_fingerprint_semantics() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = McpRuntimeState::load(dir.path());
        runtime.set_tool_snapshot("srv".into(), "fp-1".into(), vec![sample_tool("old")]);
        runtime.set_tool_snapshot("srv".into(), "fp-2".into(), vec![sample_tool("new")]);

        assert!(runtime.tool_snapshot("srv", "fp-1").is_none());
        assert_eq!(runtime.tool_snapshot("srv", "fp-2").unwrap()[0].name, "new");

        let restarted = McpRuntimeState::load(dir.path());
        assert_eq!(
            restarted.tool_snapshot("srv", "fp-2").unwrap()[0].name,
            "new"
        );
    }

    #[test]
    fn manager_depends_on_explicit_services_not_app_state() {
        let source = include_str!("manager.rs");
        assert!(!source.contains("impl AppState"));
        assert!(!source.contains("use crate::state::AppState"));
    }
}
