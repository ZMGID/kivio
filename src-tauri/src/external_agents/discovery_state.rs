//! TTL caches and single-flight permits for external CLI discovery.

#[cfg(test)]
mod tests {
    use super::*;
    fn models() -> CachedAgentModels {
        CachedAgentModels {
            models: vec![crate::external_agents::types::default_model_option()],
            source: crate::external_agents::types::ModelSource::Probed,
            reasoning_options: Vec::new(),
            reasoning_by_model: Default::default(),
            current_model: Some("configured".into()),
            current_reasoning: Some("high".into()),
            probe_error: None,
        }
    }

    #[test]
    fn model_snapshots_survive_restart_and_isolate_configurations() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("models.json");
        let state = ExternalDiscoveryState::default();
        state.save_model_snapshot(&path, "cli:project:config-a".into(), models());
        let restarted = ExternalDiscoveryState::default();
        assert_eq!(
            restarted
                .load_model_snapshot(&path, "cli:project:config-a")
                .unwrap()
                .current_reasoning
                .as_deref(),
            Some("high")
        );
        assert!(restarted
            .load_model_snapshot(&path, "cli:project:config-b")
            .is_none());
        assert!(restarted
            .get_cached_external_agent_models(
                "cli:project:config-a",
                Duration::from_secs(300),
                Duration::from_secs(30)
            )
            .is_none());
        std::fs::write(&path, "broken").unwrap();
        assert!(restarted
            .load_model_snapshot(&path, "cli:project:config-a")
            .is_none());
        restarted.save_model_snapshot(&path, "repaired".into(), models());
        assert!(restarted.load_model_snapshot(&path, "repaired").is_some());
    }

    #[test]
    fn model_snapshots_are_bounded_and_updates_replace_the_previous_entry() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("models.json");
        let state = ExternalDiscoveryState::default();
        for index in 0..=EXTERNAL_DISCOVERY_CACHE_CAPACITY {
            state.save_model_snapshot(&path, index.to_string(), models());
        }
        assert!(state.load_model_snapshot(&path, "0").is_none());
        let mut updated = models();
        updated.current_model = Some("updated".into());
        state.save_model_snapshot(&path, "1".into(), updated);
        assert_eq!(
            read_model_snapshots(&path).len(),
            EXTERNAL_DISCOVERY_CACHE_CAPACITY
        );
        assert_eq!(
            state
                .load_model_snapshot(&path, "1")
                .unwrap()
                .current_model
                .as_deref(),
            Some("updated")
        );
    }

    #[tokio::test]
    async fn invalidation_rejects_late_probes_and_concurrent_waiters_reuse_completion() {
        let state = ExternalDiscoveryState::default();
        let generation = state.model_generation();
        let permit = state.acquire_model_probe("cli:project").await;
        let waiting_since = Instant::now();
        assert!(state.set_models_if_current("cli:project".into(), models(), generation));
        drop(permit);
        let _waiter = state.acquire_model_probe("cli:project").await;
        assert!(state
            .models_completed_since("cli:project", waiting_since)
            .is_some());
        state.clear_all_external_agent_models_cache();
        assert!(!state.set_models_if_current("cli:project".into(), models(), generation));
        assert!(state
            .models_completed_since("cli:project", waiting_since)
            .is_none());
    }

    #[test]
    fn bounded_cache_sweeps_all_expired_entries_on_read() {
        let cache = Mutex::new(HashMap::from([
            (
                "expired".to_string(),
                TimedCacheEntry {
                    created_at: Instant::now() - Duration::from_secs(10),
                    last_accessed: Instant::now() - Duration::from_secs(10),
                    value: 1u32,
                },
            ),
            (
                "fresh".to_string(),
                TimedCacheEntry {
                    created_at: Instant::now(),
                    last_accessed: Instant::now() - Duration::from_secs(1),
                    value: 2u32,
                },
            ),
        ]));

        assert_eq!(get_cached(&cache, "fresh", Duration::from_secs(5)), Some(2));
        let guard = cache.lock().unwrap();
        assert_eq!(guard.len(), 1);
        assert!(!guard.contains_key("expired"));
    }
    #[test]
    fn bounded_cache_evicts_least_recently_used_entry() {
        let cache = Mutex::new(HashMap::new());
        set_cached(&cache, "a".into(), 1u32, Duration::from_secs(60), 2);
        set_cached(&cache, "b".into(), 2u32, Duration::from_secs(60), 2);
        assert_eq!(get_cached(&cache, "a", Duration::from_secs(60)), Some(1));
        set_cached(&cache, "c".into(), 3u32, Duration::from_secs(60), 2);

        let guard = cache.lock().unwrap();
        assert!(guard.contains_key("a"));
        assert!(guard.contains_key("c"));
        assert!(!guard.contains_key("b"));
    }
    #[test]
    fn bounded_cache_update_replaces_value_and_refreshes_ttl() {
        let cache = Mutex::new(HashMap::new());
        set_cached(&cache, "same".into(), 1u32, Duration::from_secs(60), 2);
        {
            let mut guard = cache.lock().unwrap();
            let entry = guard.get_mut("same").unwrap();
            entry.created_at = Instant::now() - Duration::from_secs(10);
            entry.last_accessed = entry.created_at;
        }

        set_cached(&cache, "same".into(), 2u32, Duration::from_secs(5), 2);

        assert_eq!(get_cached(&cache, "same", Duration::from_secs(5)), Some(2));
        assert_eq!(cache.lock().unwrap().len(), 1);
    }
    #[test]
    fn bounded_cache_remains_within_capacity_under_concurrent_writes() {
        let cache = Arc::new(Mutex::new(HashMap::new()));
        let threads: Vec<_> = (0..8)
            .map(|worker| {
                let cache = Arc::clone(&cache);
                std::thread::spawn(move || {
                    for item in 0..32 {
                        set_cached(
                            &cache,
                            format!("{worker}:{item}"),
                            item,
                            Duration::from_secs(60),
                            16,
                        );
                    }
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
        assert!(cache.lock().unwrap().len() <= 16);
    }
}
// Map locks never span awaits. Probe permit precedes the cache recheck.
use crate::external_agents::types::CachedAgentModels;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[derive(serde::Serialize, serde::Deserialize)]
struct ModelSnapshot {
    key: String,
    models: CachedAgentModels,
}

fn read_model_snapshots(path: &Path) -> Vec<ModelSnapshot> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}
#[derive(Debug)]
struct TimedCacheEntry<V> {
    created_at: Instant,
    last_accessed: Instant,
    value: V,
}
const EXTERNAL_DISCOVERY_CACHE_CAPACITY: usize = 64;
const EXTERNAL_DISCOVERY_CACHE_RETENTION: Duration = Duration::from_secs(300);

/// 共享的 TTL 缓存读取：每次访问全表清理过期项，命中时刷新 LRU 时间。
fn get_cached<V: Clone>(
    cache: &Mutex<HashMap<String, TimedCacheEntry<V>>>,
    key: &str,
    ttl: Duration,
) -> Option<V> {
    let mut cache = cache.lock().unwrap_or_else(|e| e.into_inner());
    cache.retain(|_, entry| entry.created_at.elapsed() <= ttl);
    let entry = cache.get_mut(key)?;
    entry.last_accessed = Instant::now();
    Some(entry.value.clone())
}

/// 共享的有界缓存写入：先清理历史项，再按 LRU 淘汰到容量以内。
fn set_cached<V>(
    cache: &Mutex<HashMap<String, TimedCacheEntry<V>>>,
    key: String,
    value: V,
    retention: Duration,
    capacity: usize,
) {
    let now = Instant::now();
    let mut cache = cache.lock().unwrap_or_else(|e| e.into_inner());
    insert_cached(&mut cache, key, value, retention, capacity, now);
}

fn insert_cached<V>(
    cache: &mut HashMap<String, TimedCacheEntry<V>>,
    key: String,
    value: V,
    retention: Duration,
    capacity: usize,
    now: Instant,
) {
    cache.retain(|_, entry| entry.created_at.elapsed() <= retention);
    cache.insert(
        key,
        TimedCacheEntry {
            created_at: now,
            last_accessed: now,
            value,
        },
    );
    while cache.len() > capacity.max(1) {
        let Some(oldest_key) = cache
            .iter()
            .min_by(|(key_a, entry_a), (key_b, entry_b)| {
                entry_a
                    .last_accessed
                    .cmp(&entry_b.last_accessed)
                    .then_with(|| entry_a.created_at.cmp(&entry_b.created_at))
                    .then_with(|| key_a.cmp(key_b))
            })
            .map(|(key, _)| key.clone())
        else {
            break;
        };
        cache.remove(&oldest_key);
    }
}

#[derive(Default)]
pub(crate) struct ExternalDiscoveryState {
    external_slash_commands_cache: Mutex<
        HashMap<
            String,
            TimedCacheEntry<Vec<crate::external_agents::types::ExternalCliSlashCommand>>,
        >,
    >,
    external_agent_models_cache:
        Mutex<HashMap<String, TimedCacheEntry<crate::external_agents::types::CachedAgentModels>>>,
    external_detected_agents_cache:
        Mutex<HashMap<String, TimedCacheEntry<Vec<crate::external_agents::types::DetectedAgent>>>>,
    availability_probe_lock: tokio::sync::Mutex<()>,
    model_probe_locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    model_generation: AtomicU64,
    model_snapshot_lock: Mutex<()>,
}
impl ExternalDiscoveryState {
    pub(crate) fn model_generation(&self) -> u64 {
        self.model_generation.load(Ordering::SeqCst)
    }

    pub(crate) fn load_model_snapshot(&self, path: &Path, key: &str) -> Option<CachedAgentModels> {
        let _guard = self
            .model_snapshot_lock
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        read_model_snapshots(path)
            .into_iter()
            .find(|s| s.key == key)
            .map(|s| s.models)
    }

    pub(crate) fn save_model_snapshot(&self, path: &Path, key: String, models: CachedAgentModels) {
        let _guard = self
            .model_snapshot_lock
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let mut snapshots = read_model_snapshots(path);
        snapshots.retain(|s| s.key != key);
        snapshots.push(ModelSnapshot { key, models });
        let excess = snapshots
            .len()
            .saturating_sub(EXTERNAL_DISCOVERY_CACHE_CAPACITY);
        snapshots.drain(..excess);
        let result = serde_json::to_string(&snapshots)
            .map_err(|e| e.to_string())
            .and_then(|text| crate::chat::storage::atomic_write(path, &text, "外部 CLI 模型快照"));
        if let Err(error) = result {
            eprintln!("[external-agent] 保存模型快照失败: {error}");
        }
    }

    pub(crate) fn set_models_if_current(
        &self,
        key: String,
        models: CachedAgentModels,
        generation: u64,
    ) -> bool {
        let mut cache = self
            .external_agent_models_cache
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if self.model_generation() != generation {
            return false;
        }
        insert_cached(
            &mut cache,
            key,
            models,
            EXTERNAL_DISCOVERY_CACHE_RETENTION,
            EXTERNAL_DISCOVERY_CACHE_CAPACITY,
            Instant::now(),
        );
        true
    }

    pub(crate) fn models_completed_since(
        &self,
        key: &str,
        started: Instant,
    ) -> Option<CachedAgentModels> {
        let cache = self
            .external_agent_models_cache
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        cache
            .get(key)
            .filter(|entry| entry.created_at >= started)
            .map(|entry| entry.value.clone())
    }
    pub(crate) async fn acquire_availability_probe(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.availability_probe_lock.lock().await
    }
    pub(crate) fn try_acquire_availability_probe(&self) -> Option<tokio::sync::MutexGuard<'_, ()>> {
        self.availability_probe_lock.try_lock().ok()
    }
    pub(crate) async fn acquire_model_probe(&self, key: &str) -> tokio::sync::OwnedMutexGuard<()> {
        self.model_probe_lock_for(key).lock_owned().await
    }
    pub fn get_cached_external_slash_commands(
        &self,
        cache_key: &str,
        full_ttl: Duration,
        empty_ttl: Duration,
    ) -> Option<Vec<crate::external_agents::types::ExternalCliSlashCommand>> {
        let mut cache = self
            .external_slash_commands_cache
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let max_ttl = full_ttl.max(empty_ttl);
        cache.retain(|_, entry| entry.created_at.elapsed() <= max_ttl);
        let entry = cache.get_mut(cache_key)?;
        let ttl = if entry.value.is_empty() {
            empty_ttl
        } else {
            full_ttl
        };
        if entry.created_at.elapsed() > ttl {
            return None;
        }
        entry.last_accessed = Instant::now();
        Some(entry.value.clone())
    }

    pub fn set_cached_external_slash_commands(
        &self,
        cache_key: String,
        commands: Vec<crate::external_agents::types::ExternalCliSlashCommand>,
    ) {
        set_cached(
            &self.external_slash_commands_cache,
            cache_key,
            commands,
            EXTERNAL_DISCOVERY_CACHE_RETENTION,
            EXTERNAL_DISCOVERY_CACHE_CAPACITY,
        );
    }

    pub fn get_cached_external_agent_models(
        &self,
        cache_key: &str,
        probed_ttl: Duration,
        fallback_ttl: Duration,
    ) -> Option<crate::external_agents::types::CachedAgentModels> {
        use crate::external_agents::types::ModelSource;
        let mut cache = self
            .external_agent_models_cache
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let max_ttl = probed_ttl.max(fallback_ttl);
        cache.retain(|_, entry| entry.created_at.elapsed() <= max_ttl);
        let entry = cache.get_mut(cache_key)?;
        let ttl = match entry.value.source {
            ModelSource::Probed => probed_ttl,
            ModelSource::Fallback => fallback_ttl,
        };
        if entry.created_at.elapsed() > ttl {
            return None;
        }
        entry.last_accessed = Instant::now();
        Some(entry.value.clone())
    }

    pub fn clear_external_agent_models_cache(&self, agent_id: &str) {
        let prefix = format!("{agent_id}:");
        let mut cache = self
            .external_agent_models_cache
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        self.model_generation.fetch_add(1, Ordering::SeqCst);
        cache.retain(|key, _| !key.starts_with(&prefix));
    }

    pub fn clear_all_external_agent_models_cache(&self) {
        let mut cache = self
            .external_agent_models_cache
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        self.model_generation.fetch_add(1, Ordering::SeqCst);
        cache.clear();
    }

    pub fn clear_detected_agents_cache(&self) {
        self.external_detected_agents_cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
    }

    pub fn get_cached_detected_agents(
        &self,
        cache_key: &str,
        ttl: Duration,
    ) -> Option<Vec<crate::external_agents::types::DetectedAgent>> {
        get_cached(&self.external_detected_agents_cache, cache_key, ttl)
    }

    pub fn set_cached_detected_agents(
        &self,
        cache_key: String,
        agents: Vec<crate::external_agents::types::DetectedAgent>,
    ) {
        set_cached(
            &self.external_detected_agents_cache,
            cache_key,
            agents,
            EXTERNAL_DISCOVERY_CACHE_RETENTION,
            EXTERNAL_DISCOVERY_CACHE_CAPACITY,
        );
    }

    fn model_probe_lock_for(&self, key: &str) -> std::sync::Arc<tokio::sync::Mutex<()>> {
        let mut locks = self
            .model_probe_locks
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        locks
            .entry(key.to_string())
            .or_insert_with(|| std::sync::Arc::new(tokio::sync::Mutex::new(())))
            .clone()
    }
}
