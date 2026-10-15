use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use crate::{agents::AgentDefinition, chat::workflow_hooks::Hook, skills::SkillRegistry};

pub(crate) struct Catalog {
    pub staging_revision: String,
    pub skills: Arc<SkillRegistry>,
    pub agents: Vec<AgentDefinition>,
    pub hooks: Vec<Hook>,
}

impl Default for Catalog {
    fn default() -> Self {
        Self {
            staging_revision: uuid::Uuid::new_v4().to_string(),
            skills: Default::default(),
            agents: vec![],
            hooks: vec![],
        }
    }
}

impl Catalog {
    pub fn discover(
        app: &tauri::AppHandle,
        conversation_id: &str,
        paths: &[String],
        cwd: Option<&std::path::Path>,
    ) -> Result<Self, String> {
        let plugins = crate::plugins::packages::discover_active()?;
        let skills = crate::skills::build_registry_with_plugins(app, paths, cwd, &plugins)?;
        let project_root = super::storage::load_conversation(app, conversation_id)
            .ok()
            .and_then(|conversation| {
                super::storage::resolve_conversation_project(app, &conversation)
                    .ok()
                    .flatten()
            })
            .and_then(|project| project.root_path)
            .map(std::path::PathBuf::from);
        let agents = crate::agents::load_agent_definitions(
            crate::agents::user_agents_dir(app).ok().as_deref(),
            project_root.as_deref(),
            &plugins,
        );
        let hooks = plugins
            .into_iter()
            .flat_map(|plugin| plugin.hooks)
            .collect();
        Ok(Self {
            skills: Arc::new(skills),
            agents,
            hooks,
            ..Default::default()
        })
    }
}

type Slot = Arc<tokio::sync::Mutex<Option<Arc<Catalog>>>>;

/// Process-local catalogs live as long as their conversations, not one model run.
/// Each slot serializes initialization/reload without blocking other conversations.
#[derive(Default)]
pub(crate) struct ResourceSessions {
    slots: Mutex<HashMap<String, Slot>>,
}

impl ResourceSessions {
    pub async fn load(
        &self,
        conversation_id: &str,
        reload: bool,
        discover: impl FnOnce() -> Result<Catalog, String> + Send + 'static,
    ) -> Result<Arc<Catalog>, String> {
        let slot = self
            .slots
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(conversation_id.to_owned())
            .or_default()
            .clone();
        let mut current = slot.lock().await;
        if !reload {
            if let Some(registry) = current.as_ref() {
                return Ok(registry.clone());
            }
        }
        let registry = tauri::async_runtime::spawn_blocking(discover)
            .await
            .map_err(|e| format!("Resource discovery failed: {e}"))??;
        if reload && registry.skills.scan_failed {
            return Err(format!(
                "Resource reload failed: {}",
                registry.skills.warnings.join("; ")
            ));
        }
        let registry = Arc::new(registry);
        *current = Some(registry.clone());
        Ok(registry)
    }

    pub fn invalidate(&self) {
        // Retire slots, including in-flight loads, after an explicit plugin change.
        self.slots.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }

    pub fn forget(&self, conversation_id: &str) {
        // In-flight loads hold the retired slot; they cannot reinsert it.
        self.slots
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(conversation_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn plugin_roles_and_hooks_share_reload_and_invalidation() {
        use crate::plugins::packages;
        use serde_json::json;
        use std::fs;
        let dir = tempfile::tempdir().unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let package = dir.path().join(&id);
        let content = package.join("content");
        fs::create_dir_all(content.join(".kivio-plugin")).unwrap();
        fs::create_dir_all(content.join("agents")).unwrap();
        fs::create_dir_all(content.join("hooks")).unwrap();
        let manifest = content.join(".kivio-plugin/plugin.json");
        fs::write(
            &manifest,
            json!({"schemaVersion":1,"name":"demo","version":"1"}).to_string(),
        )
        .unwrap();
        let mut record = json!({"id":id,"name":"demo","description":"","format":"kivio","source":"fixture","enabled":true,"components":{},"diagnostics":[]});
        fs::write(package.join("record.json"), record.to_string()).unwrap();
        let update = |version: &str| {
            fs::write(
                content.join("agents/research.md"),
                format!("---\nname: research\ndescription: Research\n---\n{version}"),
            )
            .unwrap();
            fs::write(content.join("hooks/hooks.json"), json!({"hooks":{"SessionStart":[{"hooks":[{"type":"command","command":version}]}]}}).to_string()).unwrap();
        };
        let sessions = ResourceSessions::default();
        let discover = || {
            let root = dir.path().to_path_buf();
            move || {
                let _root = packages::TestPackagesRoot::new(&root);
                let plugins = packages::discover_active()?;
                Ok(Catalog {
                    agents: crate::agents::load_agent_definitions(None, None, &plugins),
                    hooks: plugins.into_iter().flat_map(|p| p.hooks).collect(),
                    ..Default::default()
                })
            }
        };
        let check = |catalog: &Catalog, version: &str| {
            assert_eq!(catalog.hooks.len(), 1);
            assert_eq!(catalog.hooks[0].handler.command, version);
            assert_eq!(
                catalog
                    .agents
                    .iter()
                    .find(|a| a.source == "plugin")
                    .unwrap()
                    .system_prompt
                    .trim(),
                version
            );
        };
        update("first");
        let first = sessions.load("a", false, discover()).await.unwrap();
        check(&first, "first");
        update("updated");
        check(
            &sessions.load("a", false, discover()).await.unwrap(),
            "first",
        );
        check(
            &sessions.load("b", false, discover()).await.unwrap(),
            "updated",
        );
        check(
            &sessions.load("a", true, discover()).await.unwrap(),
            "updated",
        );
        fs::write(&manifest, "invalid manifest").unwrap();
        assert!(sessions.load("a", true, discover()).await.is_err());
        check(
            &sessions.load("a", false, discover()).await.unwrap(),
            "updated",
        );
        // Disabling even a broken package retires cached roles and hooks.
        record["enabled"] = json!(false);
        fs::write(package.join("record.json"), record.to_string()).unwrap();
        sessions.invalidate();
        let disabled = sessions.load("a", false, discover()).await.unwrap();
        assert!(disabled.hooks.is_empty());
        assert!(!disabled.agents.iter().any(|a| a.source == "plugin"));
        check(&first, "first"); // A running request retains its immutable snapshot.
    }

    #[tokio::test]
    async fn catalogs_survive_turns_reload_atomically_and_are_conversation_scoped() {
        let sessions = ResourceSessions::default();
        let calls = Arc::new(AtomicUsize::new(0));
        let scan = || {
            let calls = calls.clone();
            move || {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(Catalog::default())
            }
        };
        let (first, concurrent) = tokio::join!(
            sessions.load("a", false, scan()),
            sessions.load("a", false, scan())
        );
        let first = first.unwrap();
        assert!(Arc::ptr_eq(&first, &concurrent.unwrap()));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        sessions.load("b", false, scan()).await.unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert!(sessions
            .load("a", true, || Err("unreadable directory".into()))
            .await
            .is_err());
        assert!(Arc::ptr_eq(
            &first,
            &sessions.load("a", false, scan()).await.unwrap()
        ));
        let refreshed = sessions.load("a", true, scan()).await.unwrap();
        assert!(!Arc::ptr_eq(&first, &refreshed));
        sessions.forget("a");
        assert!(!Arc::ptr_eq(
            &refreshed,
            &sessions.load("a", false, scan()).await.unwrap()
        ));
        assert_eq!(calls.load(Ordering::SeqCst), 4);
    }
    #[tokio::test]
    async fn deleting_a_conversation_retires_an_inflight_catalog() {
        let sessions = Arc::new(ResourceSessions::default());
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let old = sessions.clone();
        let pending = tokio::spawn(async move {
            old.load("removed", false, move || {
                started_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                Ok(Catalog::default())
            })
            .await
            .unwrap()
        });
        started_rx.await.unwrap();
        sessions.forget("removed");
        let fresh = sessions
            .load("removed", false, || Ok(Catalog::default()))
            .await
            .unwrap();
        release_tx.send(()).unwrap();
        let retired = pending.await.unwrap();
        let current = sessions
            .load("removed", false, || panic!("must reuse current snapshot"))
            .await
            .unwrap();
        assert!(Arc::ptr_eq(&fresh, &current));
        assert!(!Arc::ptr_eq(&retired, &current));
    }
}
