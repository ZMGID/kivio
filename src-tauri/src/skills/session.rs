use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use super::SkillRegistry;

type Slot = Arc<tokio::sync::Mutex<Option<Arc<SkillRegistry>>>>;

/// Process-local catalogs live as long as their conversations, not one model run.
/// Each slot serializes initialization/reload without blocking other conversations.
#[derive(Default)]
pub(crate) struct SkillSessions {
    slots: Mutex<HashMap<String, Slot>>,
}

impl SkillSessions {
    pub async fn load(
        &self,
        conversation_id: &str,
        reload: bool,
        discover: impl FnOnce() -> Result<SkillRegistry, String> + Send + 'static,
    ) -> Result<Arc<SkillRegistry>, String> {
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
            .map_err(|e| format!("Skill discovery failed: {e}"))??;
        if reload && registry.scan_failed {
            return Err(format!(
                "Skill reload failed: {}",
                registry.warnings.join("; ")
            ));
        }
        let registry = Arc::new(registry);
        *current = Some(registry.clone());
        Ok(registry)
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
    async fn catalogs_survive_turns_reload_atomically_and_are_conversation_scoped() {
        let sessions = SkillSessions::default();
        let calls = Arc::new(AtomicUsize::new(0));
        let scan = || {
            let calls = calls.clone();
            move || {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(SkillRegistry::default())
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
        let sessions = Arc::new(SkillSessions::default());
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let old = sessions.clone();
        let pending = tokio::spawn(async move {
            old.load("removed", false, move || {
                started_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                Ok(SkillRegistry::default())
            })
            .await
            .unwrap()
        });
        started_rx.await.unwrap();
        sessions.forget("removed");
        let fresh = sessions
            .load("removed", false, || Ok(SkillRegistry::default()))
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
