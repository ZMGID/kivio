//! Tauri adapter for the shared, Tokio-owned Apple Vision helper lifecycle.
#[cfg(target_os = "macos")]
#[path = "macos_ocr_runtime.rs"]
mod runtime;
#[cfg(target_os = "macos")]
use std::sync::Arc;
#[cfg(target_os = "macos")]
pub struct MacOcrClient {
    inner: Arc<runtime::MacOcrClient>,
}
#[cfg(target_os = "macos")]
impl MacOcrClient {
    pub fn new(_: &tauri::AppHandle) -> Arc<Self> {
        let inner = runtime::MacOcrClient::bundled_helper()
            .map(runtime::MacOcrClient::new)
            .unwrap_or_else(|_| runtime::MacOcrClient::headless());
        Arc::new(Self { inner })
    }
    pub fn headless() -> Arc<Self> {
        Arc::new(Self {
            inner: runtime::MacOcrClient::headless(),
        })
    }
    #[cfg(test)]
    pub fn disabled() -> Arc<Self> {
        Self::headless()
    }
    pub fn shutdown(&self) {
        self.inner.shutdown();
    }
    pub async fn ocr_image(self: &Arc<Self>, path: &str) -> Result<String, String> {
        self.inner.ocr_image(path).await
    }
}
