#[path = "../../../src-tauri/src/offline_models/ort_runtime.rs"]
mod ort_runtime;
pub use ort_runtime::{ensure_ort_init, init_ort_from_model_dir};
