use std::{
    collections::HashMap,
    future::Future,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use serde::Deserialize;
use serde_json::{json, Value};

use super::super::{
    overrides::{LaunchIdentity, LaunchOwner},
    types::RuntimeAgentDef,
};
use super::{CodexResetTarget, PersistedQuota, ProbeReading};

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
    completed: Mutex<Option<Reading>>,
}

struct Reading {
    refreshed: Option<Instant>,
    value: Value,
}

impl Slot {
    fn snapshot(&self, mode: AccountSnapshotMode, requested: Instant) -> Option<Value> {
        let completed = self.completed.lock().unwrap_or_else(|err| err.into_inner());
        let reading = completed.as_ref()?;
        (match mode {
            AccountSnapshotMode::Cached => true,
            AccountSnapshotMode::Automatic => reading
                .refreshed
                .is_some_and(|time| time.elapsed() < AUTOMATIC_REFRESH_INTERVAL),
            AccountSnapshotMode::Refresh => reading.refreshed.is_some_and(|time| time >= requested),
        })
        .then(|| reading.value.clone())
    }

    fn restore(&self, value: Value) {
        let mut completed = self.completed.lock().unwrap_or_else(|err| err.into_inner());
        completed.get_or_insert(Reading {
            refreshed: None,
            value,
        });
    }

    fn publish(&self, agent: &str, reading: Result<ProbeReading, String>) -> Value {
        let mut completed = self.completed.lock().unwrap_or_else(|err| err.into_inner());
        let (mut next, failed) = match reading {
            Ok(reading) => {
                let failed = reading.snapshot["unavailable"].is_string()
                    && !reading.definitively_unavailable;
                (reading.snapshot, failed)
            }
            Err(error) => (super::unavailable_snapshot(agent, &error), true),
        };
        if failed {
            if let Some(previous) =
                completed
                    .as_ref()
                    .map(|reading| &reading.value)
                    .filter(|previous| {
                        (previous["windows"]
                            .as_array()
                            .is_some_and(|w| !w.is_empty())
                            || previous["resetCredits"]["availableCount"]
                                .as_u64()
                                .is_some_and(|count| count > 0))
                            && same_reported_account(previous, &next)
                    })
            {
                let failure = next["unavailable"].take();
                next = previous.clone();
                next["refreshError"] = failure;
            }
        }
        next["nextRefreshAt"] = json!(
            chrono::Utc::now().timestamp_millis() + AUTOMATIC_REFRESH_INTERVAL.as_millis() as i64
        );
        *completed = Some(Reading {
            refreshed: Some(Instant::now()),
            value: next.clone(),
        });
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

#[derive(Clone, PartialEq, Eq, Hash)]
pub(in crate::external_agents) enum AccountLaunch {
    Identified(LaunchOwner),
    Unverified(LaunchIdentity),
}

impl AccountLaunch {
    fn new(identity: LaunchIdentity, disk: Option<&PersistedQuota>) -> Self {
        if disk.is_some() {
            Self::Identified(identity.owner().clone())
        } else {
            Self::Unverified(identity)
        }
    }

    fn owner(&self) -> &LaunchOwner {
        match self {
            Self::Identified(owner) => owner,
            Self::Unverified(identity) => identity.owner(),
        }
    }

    pub(in crate::external_agents) fn ensure_current(
        &self,
        def: &RuntimeAgentDef,
        bin: &Path,
    ) -> Result<(), String> {
        match self {
            Self::Identified(owner) => owner.ensure_current(def, bin),
            Self::Unverified(identity) => identity.ensure_current(def, bin),
        }
    }
}

type SlotKey = (AccountLaunch, PathBuf, Option<String>);

/// The desktop process owns quotas across webviews. Project-sensitive profiles
/// stay isolated by cwd, and native launch changes make old snapshots unreachable.
#[derive(Default)]
pub(crate) struct AccountSnapshots {
    slots: Mutex<HashMap<SlotKey, (Instant, Arc<Slot>)>>,
    pub(super) pending_codex_resets: tokio::sync::Mutex<HashMap<CodexResetTarget, uuid::Uuid>>,
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
        let (identity, owner, slot) = {
            let slots = self.slots.lock().unwrap_or_else(|err| err.into_inner());
            let ((identity, _, owner), (_, slot)) = slots
                .iter()
                .filter(|((identity, scope, _), _)| {
                    scope == cwd && identity.owner().binary_for(def.id).is_some()
                })
                .max_by_key(|(_, (accessed, _))| *accessed)?;
            (identity.clone(), owner.clone(), slot.clone())
        };
        let snapshot = slot.snapshot(mode, requested)?;
        let binary = identity.owner().binary_for(def.id)?;
        if identity.ensure_current(def, binary).is_err() {
            return None;
        }
        let current = PersistedQuota::for_cli(None, def, binary, cwd);
        (current.as_ref().map(PersistedQuota::owner) == owner.as_deref()).then_some(snapshot)
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
        key: (LaunchIdentity, PathBuf),
        mode: AccountSnapshotMode,
        requested: Instant,
        validate: impl Fn(&AccountLaunch) -> Result<(), String>,
        probe: impl FnOnce() -> F,
        disk: Option<&PersistedQuota>,
    ) -> Result<(Option<Value>, bool), String>
    where
        F: Future<Output = Result<ProbeReading, String>>,
    {
        let launch = AccountLaunch::new(key.0, disk);
        let validate = || validate(&launch);
        validate()?;
        let key = (
            launch.clone(),
            key.1,
            disk.map(|disk| disk.owner().to_owned()),
        );
        let slot = match self.slot(key.clone(), AccountSnapshotMode::Cached) {
            Some(slot) => slot,
            None => {
                let restored = disk.and_then(PersistedQuota::restore);
                let slot_mode = if restored.is_some() {
                    AccountSnapshotMode::Automatic
                } else {
                    mode
                };
                let Some(slot) = self.slot(key, slot_mode) else {
                    return Ok((None, false));
                };
                if let Some(value) = restored {
                    validate()?;
                    slot.restore(value);
                }
                slot
            }
        };
        let matches_owner = || disk.is_none_or(PersistedQuota::matches_current_profile);
        let snapshot = slot.snapshot(mode, requested);
        if snapshot.is_some() || mode == AccountSnapshotMode::Cached {
            return Ok((snapshot.filter(|_| matches_owner()), false));
        }
        let _refresh = slot.refresh.lock().await;
        validate()?;
        if !matches_owner() {
            return Ok((None, false));
        }
        let snapshot = slot.snapshot(mode, requested);
        if snapshot.is_some() {
            return Ok((snapshot, false));
        }
        let reading = probe().await;
        validate()?;
        if !matches_owner() {
            return Ok((None, false));
        }
        let removed = reading
            .as_ref()
            .is_ok_and(|reading| reading.definitively_unavailable);
        let value = slot.publish(agent, reading);
        if let Some(disk) = disk {
            if removed {
                disk.invalidate();
            } else if value["unavailable"].is_null() && value["refreshError"].is_null() {
                disk.save(&value);
            }
        }
        Ok((Some(value), true))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::external_agents::registry::get_agent_def;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    fn key(scope: &str) -> (LaunchIdentity, PathBuf) {
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
                |_| Ok(()),
                || async {
                    probes.fetch_add(1, Ordering::Relaxed);
                    tokio::task::yield_now().await;
                    Ok(report("a@example.test", 65, 100))
                },
                None,
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
                |_| Ok(()),
                || std::future::ready(next),
                None,
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
                |_| Ok(()),
                || async { panic!("automatic failure retry must honor the interval") },
                None,
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
                |_| {
                    current
                        .load(Ordering::Relaxed)
                        .then_some(())
                        .ok_or("profile changed".into())
                },
                || async {
                    current.store(false, Ordering::Relaxed);
                    Ok(report("a@example.test", 60, 100))
                },
                None,
            )
            .await;
        assert_eq!(rejected.unwrap_err(), "profile changed");
        let peek = |scope| {
            cache.read(
                "antigravity",
                key(scope),
                AccountSnapshotMode::Cached,
                Instant::now(),
                |_| Ok(()),
                || async { panic!("cache reads must never probe") },
                None,
            )
        };
        assert_eq!(peek("first").await.unwrap(), (None, false));
        cache
            .read(
                "antigravity",
                key("first"),
                AccountSnapshotMode::Refresh,
                Instant::now(),
                |_| Ok(()),
                || async { Ok(report("b@example.test", 40, 200)) },
                None,
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
