//! One initialize response feeds both picker catalogs for two minutes.
use crate::HarnessError;
use serde_json::Value;
use std::{future::Future, time::Duration};
use tokio::time::Instant;

#[derive(Default)]
pub(super) struct InitializeCache {
    state: tokio::sync::Mutex<State>,
}
#[derive(Default)]
struct State {
    context: Option<[u8; 32]>,
    completed_at: Option<Instant>,
    response: Option<Result<Value, String>>,
}
impl InitializeCache {
    pub(super) async fn get<F, Fut>(
        &self,
        context: impl Fn() -> Result<[u8; 32], HarnessError>,
        probe: F,
    ) -> Result<Value, HarnessError>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<Value, HarnessError>>,
    {
        let requested_at = Instant::now();
        let mut state = self.state.lock().await;
        let key = context()?;
        if state.context != Some(key) {
            *state = State {
                context: Some(key),
                ..Default::default()
            };
        }
        if let (Some(at), Some(response)) = (state.completed_at, &state.response)
            && (at >= requested_at || (response.is_ok() && at.elapsed() < Duration::from_secs(120)))
        {
            return response.clone().map_err(HarnessError::Protocol);
        }
        let response = probe().await.map_err(|error| error.to_string());
        if context()? != key {
            *state = State::default();
            return Err(HarnessError::Protocol(
                "Claude credentials changed during initialize; retry".into(),
            ));
        }
        state.completed_at = Some(Instant::now());
        state.response = Some(response.clone());
        response.map_err(HarnessError::Protocol)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
    #[tokio::test(start_paused = true)]
    async fn initialize_coalesces_expires_and_invalidates_with_credentials() {
        let cache = InitializeCache::default();
        let calls = AtomicUsize::new(0);
        let probe = || async {
            calls.fetch_add(1, Relaxed);
            tokio::time::sleep(Duration::from_millis(1)).await;
            Ok(serde_json::json!({"models":[],"commands":[]}))
        };
        let results =
            futures::future::join_all((0..10).map(|_| cache.get(|| Ok([1; 32]), probe))).await;
        assert!(results.iter().all(Result::is_ok));
        assert_eq!(calls.load(Relaxed), 1);
        tokio::time::advance(Duration::from_secs(119)).await;
        cache.get(|| Ok([1; 32]), probe).await.unwrap();
        assert_eq!(calls.load(Relaxed), 1);
        tokio::time::advance(Duration::from_secs(2)).await;
        cache.get(|| Ok([1; 32]), probe).await.unwrap();
        assert_eq!(calls.load(Relaxed), 2);
        cache.get(|| Ok([2; 32]), probe).await.unwrap();
        assert_eq!(calls.load(Relaxed), 3);
    }
    #[tokio::test]
    async fn failed_initialize_retries_and_raced_login_is_not_cached() {
        let cache = InitializeCache::default();
        assert!(
            cache
                .get(
                    || Ok([1; 32]),
                    || async { Err(HarnessError::Protocol("offline".into())) }
                )
                .await
                .is_err()
        );
        assert!(
            cache
                .get(|| Ok([1; 32]), || async { Ok(Value::Null) })
                .await
                .is_ok()
        );
        let key = AtomicUsize::new(2);
        assert!(
            cache
                .get(
                    || Ok([key.load(Relaxed) as u8; 32]),
                    || async {
                        key.store(3, Relaxed);
                        Ok(Value::Null)
                    }
                )
                .await
                .is_err()
        );
    }
}
