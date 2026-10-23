//! Process-local provider state shared with the model runtime, plus image routing.
use crate::{chat::image_generation::ImageRoute, settings::Settings};
use std::{collections::HashMap, sync::Mutex};
#[path = "provider_request_runtime.rs"]
pub mod requests;
pub use requests::ProviderRequestRuntime;
#[derive(Default)]
pub(crate) struct ProviderRuntimeState {
    requests: ProviderRequestRuntime,
    image_route_cache: Mutex<HashMap<(String, String), ImageRoute>>,
}
impl std::ops::Deref for ProviderRuntimeState {
    type Target = ProviderRequestRuntime;
    fn deref(&self) -> &Self::Target {
        &self.requests
    }
}
impl ProviderRuntimeState {
    pub(crate) fn new(settings: &Settings) -> Self {
        Self {
            requests: ProviderRequestRuntime::new(&settings.providers),
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

    pub fn sync_preferred_api_keys(&self, previous: &Settings, next: &Settings) {
        self.requests
            .sync_preferred_api_keys(&previous.providers, &next.providers);
    }
}
