//! Offline model host adapters; manifest, validation and downloads are shared.
include!("offline_models/core.rs");
#[path = "offline_models/ort_runtime.rs"]
mod ort_runtime;
pub use ort_runtime::{ensure_ort_init, init_ort_from_model_dir};
use tauri::{AppHandle, Emitter, Manager};

impl OfflineModelManager {
    pub fn new(app: &AppHandle, http: reqwest::Client) -> Arc<Self> {
        let directory = app
            .path()
            .app_data_dir()
            .map(|dir| dir.join("rapidocr-models"))
            .map_err(|e| e.to_string());
        let app = app.clone();
        Self::with_progress(
            directory,
            http,
            Some(Arc::new(move |progress| {
                let _ = app.emit("replace-translation-pack-progress", progress);
            })),
        )
    }
}
