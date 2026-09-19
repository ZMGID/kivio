//! Process-local provider failover and learned endpoint capabilities.
//! Callers never hold these locks; no operation performs I/O or awaits.
use crate::{chat::image_generation::ImageRoute, settings::Settings};
use std::{
    collections::{HashMap, HashSet},
    sync::Mutex,
    time::{Duration, Instant},
};
const KEY_COOLDOWN: Duration = Duration::from_secs(60);
#[derive(Default)]
pub(crate) struct ProviderRuntimeState {
    key_cooldowns: Mutex<HashMap<(String, usize), Instant>>,
    active_key_idx: Mutex<HashMap<String, usize>>,
    prompt_cache_key_unsupported: Mutex<HashSet<String>>,
    prompt_cache_retention_unsupported: Mutex<HashSet<String>>,
    reasoning_replay_unsupported: Mutex<HashSet<String>>,
    image_route_cache: Mutex<HashMap<(String, String), ImageRoute>>,
}
impl ProviderRuntimeState {
    pub(crate) fn new(settings: &Settings) -> Self {
        Self {
            active_key_idx: Mutex::new(
                settings
                    .providers
                    .iter()
                    .map(|p| (p.id.clone(), p.clamped_active_key_index()))
                    .collect(),
            ),
            ..Self::default()
        }
    }
    pub(crate) fn image_route(&self, key: &(String, String)) -> Option<ImageRoute> {
        self.image_route_cache
            .lock()
            .ok()
            .and_then(|cache| cache.get(key).copied())
    }
    pub(crate) fn remember_image_route(&self, key: (String, String), route: ImageRoute) {
        if let Ok(mut cache) = self.image_route_cache.lock() {
            cache.insert(key, route);
        }
    }
    pub fn pick_active_key(
        &self,
        provider_id: &str,
        total: usize,
        tried: &HashSet<usize>,
    ) -> Option<usize> {
        if total == 0 {
            return None;
        }
        let now = Instant::now();
        let cooldowns = self.key_cooldowns.lock().unwrap_or_else(|e| e.into_inner());
        let active = self
            .active_key_idx
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(provider_id)
            .copied()
            .unwrap_or(0)
            .min(total.saturating_sub(1));

        let in_cooldown = |idx: usize| {
            cooldowns
                .get(&(provider_id.to_string(), idx))
                .map(|until| *until > now)
                .unwrap_or(false)
        };

        // 1) 优先 active idx（未试过 + 未冷却）
        if !tried.contains(&active) && !in_cooldown(active) {
            return Some(active);
        }
        // 2) 从 active+1 开始环绕扫描
        for offset in 1..total {
            let idx = (active + offset) % total;
            if !tried.contains(&idx) && !in_cooldown(idx) {
                return Some(idx);
            }
        }
        // 3) 全部冷却 → 兜底找一个未试过的（无视冷却，避免完全无 key 可用）
        for offset in 0..total {
            let idx = (active + offset) % total;
            if !tried.contains(&idx) {
                return Some(idx);
            }
        }
        None
    }

    pub fn prompt_cache_key_unsupported(&self, base_url: &str) -> bool {
        self.prompt_cache_key_unsupported
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains(base_url)
    }

    pub fn mark_prompt_cache_key_unsupported(&self, base_url: &str) {
        self.prompt_cache_key_unsupported
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(base_url.to_string());
    }

    pub fn reasoning_replay_unsupported(&self, base_url: &str) -> bool {
        self.reasoning_replay_unsupported
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains(base_url)
    }

    pub fn mark_reasoning_replay_unsupported(&self, base_url: &str) {
        self.reasoning_replay_unsupported
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(base_url.to_string());
    }

    pub fn prompt_cache_retention_unsupported(&self, base_url: &str) -> bool {
        self.prompt_cache_retention_unsupported
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains(base_url)
    }

    pub fn mark_prompt_cache_retention_unsupported(&self, base_url: &str) {
        self.prompt_cache_retention_unsupported
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(base_url.to_string());
    }

    pub fn mark_key_failed(&self, provider_id: &str, idx: usize) {
        let mut cooldowns = self.key_cooldowns.lock().unwrap_or_else(|e| e.into_inner());
        cooldowns.insert(
            (provider_id.to_string(), idx),
            Instant::now() + KEY_COOLDOWN,
        );
    }

    pub fn mark_key_ok(&self, provider_id: &str, idx: usize) {
        let mut cooldowns = self.key_cooldowns.lock().unwrap_or_else(|e| e.into_inner());
        cooldowns.remove(&(provider_id.to_string(), idx));
        drop(cooldowns);
        let mut active = self
            .active_key_idx
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        active.insert(provider_id.to_string(), idx);
    }

    pub fn prefer_key(&self, provider_id: &str, idx: usize) {
        let mut cooldowns = self.key_cooldowns.lock().unwrap_or_else(|e| e.into_inner());
        cooldowns.retain(|(id, _), _| id != provider_id);
        drop(cooldowns);
        let mut active = self
            .active_key_idx
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        active.insert(provider_id.to_string(), idx);
    }

    pub fn sync_preferred_api_keys(&self, previous: &Settings, next: &Settings) {
        let next_ids: std::collections::HashSet<&str> =
            next.providers.iter().map(|p| p.id.as_str()).collect();
        {
            let mut active = self
                .active_key_idx
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            active.retain(|id, _| next_ids.contains(id.as_str()));
        }
        {
            let mut cooldowns = self.key_cooldowns.lock().unwrap_or_else(|e| e.into_inner());
            cooldowns.retain(|(id, _), _| next_ids.contains(id.as_str()));
        }
        for provider in &next.providers {
            let preferred = provider.clamped_active_key_index();
            let changed = previous
                .providers
                .iter()
                .find(|old| old.id == provider.id)
                .map(|old| {
                    old.api_keys != provider.api_keys
                        || old.active_key_index != provider.active_key_index
                })
                .unwrap_or(true);
            if changed {
                self.prefer_key(&provider.id, preferred);
            }
        }
    }
}
