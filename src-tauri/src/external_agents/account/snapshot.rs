use std::{
    collections::HashMap,
    future::Future,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use serde::Deserialize;
use serde_json::{json, Value};

use super::super::{overrides::LaunchIdentity, types::RuntimeAgentDef};
use super::ProbeReading;

const AUTOMATIC_REFRESH_INTERVAL: Duration = Duration::from_secs(300);
const CAPACITY: usize = 64;

#[derive(Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) enum AccountSnapshotMode {
    Cached,
    #[default]
    Automatic,
    Refresh,
}

#[derive(Default)]
struct Slot {
    refresh: tokio::sync::Mutex<()>,
    completed: Mutex<Option<(Instant, Value)>>,
}

impl Slot {
    fn snapshot(&self, mode: AccountSnapshotMode, requested: Instant) -> Option<Value> {
        let completed = self.completed.lock().unwrap_or_else(|err| err.into_inner());
        let (time, value) = completed.as_ref()?;
        (match mode {
            AccountSnapshotMode::Cached => true,
            AccountSnapshotMode::Automatic => time.elapsed() < AUTOMATIC_REFRESH_INTERVAL,
            AccountSnapshotMode::Refresh => *time >= requested,
        })
        .then(|| value.clone())
    }

    fn publish(&self, agent: &str, reading: Result<ProbeReading, String>) -> Value {
        let mut completed = self.completed.lock().unwrap_or_else(|err| err.into_inner());
        let (mut next, failed) = match reading {
            Ok(reading) => {
                let failed = reading.snapshot["unavailable"].is_string()
                    && !reading.definitively_unavailable;
                (reading.snapshot, failed)
            }
            Err(error) => (
                json!({"agentId":agent,"account":null,"windows":[],"credits":null,
                    "unavailable":error,"checkedAt":chrono::Utc::now().timestamp()}),
                true,
            ),
        };
        if failed {
            if let Some((_, previous)) = completed.as_ref().filter(|(_, previous)| {
                previous["windows"]
                    .as_array()
                    .is_some_and(|w| !w.is_empty())
                    && same_reported_account(previous, &next)
            }) {
                let failure = next["unavailable"].take();
                next = previous.clone();
                next["refreshError"] = failure;
            }
        }
        next["nextRefreshAt"] = json!(
            chrono::Utc::now().timestamp_millis() + AUTOMATIC_REFRESH_INTERVAL.as_millis() as i64
        );
        *completed = Some((Instant::now(), next.clone()));
        next
    }
}

fn same_reported_account(previous: &Value, next: &Value) -> bool {
    ["type", "email"].iter().all(|key| {
        match (
            previous["account"][key].as_str(),
            next["account"][key].as_str(),
        ) {
            (Some(previous), Some(next)) => previous.eq_ignore_ascii_case(next),
            _ => true,
        }
    })
}

type SlotKey = (LaunchIdentity, PathBuf);

/// The desktop process owns quotas across webviews. Project-sensitive profiles
/// stay isolated by cwd, and native launch changes make old snapshots unreachable.
#[derive(Default)]
pub(crate) struct AccountSnapshots {
    slots: Mutex<HashMap<SlotKey, (Instant, Arc<Slot>)>>,
}

impl AccountSnapshots {
    /// Revalidate the recorded launch without resolving the CLI on PATH again.
    pub(in crate::external_agents) fn peek(
        &self,
        def: &RuntimeAgentDef,
        cwd: &Path,
        mode: AccountSnapshotMode,
        requested: Instant,
    ) -> Option<Value> {
        let (identity, slot) = {
            let slots = self.slots.lock().unwrap_or_else(|err| err.into_inner());
            let ((identity, _), (_, slot)) = slots
                .iter()
                .filter(|((identity, scope), _)| {
                    scope == cwd && identity.binary_for(def.id).is_some()
                })
                .max_by_key(|(_, (accessed, _))| *accessed)?;
            (identity.clone(), slot.clone())
        };
        let snapshot = slot.snapshot(mode, requested)?;
        identity
            .still_current(def, identity.binary_for(def.id)?)
            .then_some(snapshot)
    }

    fn slot(&self, key: SlotKey, mode: AccountSnapshotMode) -> Option<Arc<Slot>> {
        let mut slots = self.slots.lock().unwrap_or_else(|err| err.into_inner());
        if let Some((accessed, slot)) = slots.get_mut(&key) {
            *accessed = Instant::now();
            return Some(slot.clone());
        }
        if mode == AccountSnapshotMode::Cached {
            return None;
        }
        if slots.len() >= CAPACITY {
            let oldest = slots
                .iter()
                .filter(|(_, (_, slot))| Arc::strong_count(slot) == 1)
                .min_by_key(|(_, (accessed, _))| *accessed)
                .map(|(key, _)| key.clone());
            if let Some(oldest) = oldest {
                slots.remove(&oldest);
            }
        }
        let slot = Arc::new(Slot::default());
        slots.insert(key, (Instant::now(), slot.clone()));
        Some(slot)
    }

    pub(in crate::external_agents) async fn read<F>(
        &self,
        agent: &str,
        key: SlotKey,
        mode: AccountSnapshotMode,
        requested: Instant,
        validate: impl Fn() -> Result<(), String>,
        probe: impl FnOnce() -> F,
    ) -> Result<(Option<Value>, bool), String>
    where
        F: Future<Output = Result<ProbeReading, String>>,
    {
        validate()?;
        let Some(slot) = self.slot(key, mode) else {
            return Ok((None, false));
        };
        let snapshot = slot.snapshot(mode, requested);
        if snapshot.is_some() || mode == AccountSnapshotMode::Cached {
            return Ok((snapshot, false));
        }
        let _refresh = slot.refresh.lock().await;
        validate()?;
        let snapshot = slot.snapshot(mode, requested);
        if snapshot.is_some() {
            return Ok((snapshot, false));
        }
        let reading = probe().await;
        validate()?;
        Ok((Some(slot.publish(agent, reading)), true))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::external_agents::registry::get_agent_def;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    fn key(scope: &str) -> SlotKey {
        let def = get_agent_def("antigravity").unwrap();
        (
            LaunchIdentity::capture(def, &PathBuf::from("agy-fixture")),
            scope.into(),
        )
    }

    fn report(email: &str, used: u64, checked_at: u64) -> ProbeReading {
        ProbeReading {
            snapshot: json!({"agentId":"antigravity","account":{"email":email},"windows":[
                {"name":"Weekly","usedPercent":used}],"unavailable":null,"checkedAt":checked_at}),
            definitively_unavailable: false,
        }
    }

    #[tokio::test]
    async fn concurrent_manual_and_automatic_reads_share_one_probe() {
        let cache = AccountSnapshots::default();
        let probes = AtomicUsize::new(0);
        let requested = Instant::now();
        let refresh = |mode| {
            cache.read(
                "antigravity",
                key("default"),
                mode,
                requested,
                || Ok(()),
                || async {
                    probes.fetch_add(1, Ordering::Relaxed);
                    tokio::task::yield_now().await;
                    Ok(report("a@example.test", 65, 100))
                },
            )
        };
        let (first, second, third) = tokio::join!(
            refresh(AccountSnapshotMode::Refresh),
            refresh(AccountSnapshotMode::Automatic),
            refresh(AccountSnapshotMode::Refresh)
        );
        let (published, updated) = first.unwrap();
        assert!(updated);
        assert_eq!(second.unwrap(), (published.clone(), false));
        assert_eq!(
            cache.peek(
                get_agent_def("antigravity").unwrap(),
                Path::new("default"),
                AccountSnapshotMode::Automatic,
                Instant::now(),
            ),
            published
        );
        assert!(published.as_ref().unwrap()["nextRefreshAt"].is_i64());
        assert_eq!(third.unwrap(), (published, false));
        assert_eq!(probes.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn failing_refresh_keeps_original_reading_until_account_changes() {
        let cache = AccountSnapshots::default();
        let refresh = |next| {
            cache.read(
                "antigravity",
                key("default"),
                AccountSnapshotMode::Refresh,
                Instant::now(),
                || Ok(()),
                || std::future::ready(next),
            )
        };
        refresh(Ok(report("a@example.test", 100, 100)))
            .await
            .unwrap();
        let (stale, _) = refresh(Err("upstream timed out".into())).await.unwrap();
        let stale = stale.unwrap();
        assert_eq!(stale["windows"][0]["usedPercent"], 100);
        assert_eq!(stale["checkedAt"], 100);
        assert_eq!(stale["refreshError"], "upstream timed out");

        let (cached, _) = cache
            .read(
                "antigravity",
                key("default"),
                AccountSnapshotMode::Automatic,
                Instant::now(),
                || Ok(()),
                || async { panic!("automatic failure retry must honor the interval") },
            )
            .await
            .unwrap();
        assert_eq!(cached, Some(stale));

        let changed_account = ProbeReading {
            snapshot: json!({"account":{"email":"b@example.test"},"windows":[],
                "unavailable":"quota temporarily unavailable","checkedAt":200}),
            definitively_unavailable: false,
        };
        let (changed, _) = refresh(Ok(changed_account)).await.unwrap();
        let changed = changed.unwrap();
        assert!(changed["windows"].as_array().unwrap().is_empty());
        assert!(changed["refreshError"].is_null());
        assert_eq!(changed["account"]["email"], "b@example.test");
    }

    #[tokio::test]
    async fn reads_isolate_workspace_and_reject_late_results_after_launch_changes() {
        let cache = AccountSnapshots::default();
        let current = AtomicBool::new(true);
        let rejected = cache
            .read(
                "antigravity",
                key("first"),
                AccountSnapshotMode::Refresh,
                Instant::now(),
                || {
                    current
                        .load(Ordering::Relaxed)
                        .then_some(())
                        .ok_or("profile changed".into())
                },
                || async {
                    current.store(false, Ordering::Relaxed);
                    Ok(report("a@example.test", 60, 100))
                },
            )
            .await;
        assert_eq!(rejected.unwrap_err(), "profile changed");
        let peek = |scope| {
            cache.read(
                "antigravity",
                key(scope),
                AccountSnapshotMode::Cached,
                Instant::now(),
                || Ok(()),
                || async { panic!("cache reads must never probe") },
            )
        };
        assert_eq!(peek("first").await.unwrap(), (None, false));
        cache
            .read(
                "antigravity",
                key("first"),
                AccountSnapshotMode::Refresh,
                Instant::now(),
                || Ok(()),
                || async { Ok(report("b@example.test", 40, 200)) },
            )
            .await
            .unwrap();
        assert_eq!(peek("second").await.unwrap(), (None, false));
        assert_eq!(
            peek("first").await.unwrap().0.unwrap()["account"]["email"],
            "b@example.test"
        );
    }
}
