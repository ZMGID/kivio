//! TTL caches and single-flight permits for external CLI discovery.

#[cfg(test)]
mod tests {
    use super::*;
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
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
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
}
impl ExternalDiscoveryState {
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

    pub fn set_cached_external_agent_models(
        &self,
        cache_key: String,
        models: crate::external_agents::types::CachedAgentModels,
    ) {
        set_cached(
            &self.external_agent_models_cache,
            cache_key,
            models,
            EXTERNAL_DISCOVERY_CACHE_RETENTION,
            EXTERNAL_DISCOVERY_CACHE_CAPACITY,
        );
    }

    pub fn clear_external_agent_models_cache(&self, agent_id: &str) {
        let prefix = format!("{agent_id}:");
        self.external_agent_models_cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|key, _| !key.starts_with(&prefix));
    }

    pub fn clear_all_external_agent_models_cache(&self) {
        self.external_agent_models_cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
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
