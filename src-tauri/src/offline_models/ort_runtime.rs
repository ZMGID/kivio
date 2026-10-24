use super::*;
use tokio::sync::OnceCell;
#[cfg(target_os = "windows")]
use std::os::windows::ffi::OsStrExt;
#[cfg(target_os = "windows")]
use windows::{Win32::System::LibraryLoader::{LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW}, core::PCWSTR};

static ORT_INIT: OnceCell<()> = OnceCell::const_new();

pub async fn ensure_ort_init(manager: &OfflineModelManager) -> Result<(), String> {
    let dir = manager.model_dir()?;
    if runtime_files().iter().any(|file| {
        manager.validation_state(&dir.join(file.relative_path), file) != ValidationState::Ready
    }) {
        return Err("onnxruntime_missing".into());
    }
    ORT_INIT
        .get_or_try_init(|| async {
            init_ort_from_model_dir(&dir)?;
            Ok::<_, String>(())
        })
        .await?;
    Ok(())
}

/// Initialize ONNX Runtime from an already verified model directory.
///
/// The GUI uses [`ensure_ort_init`] for its process-wide singleton. The one-shot RapidOCR worker
/// calls this directly because it owns a fresh process and exits immediately after one inference.
pub fn init_ort_from_model_dir(dir: &Path) -> Result<(), String> {
    prepare_onnxruntime_dll_dir(dir)?;
    ort::init_from(dir.join(ONNX_RUNTIME_FILE_NAME))
        .map_err(|e| format!("ort init_from failed: {e}"))?
        .commit();
    Ok(())
}

#[cfg(target_os = "windows")]
fn prepare_onnxruntime_dll_dir(dir: &Path) -> Result<(), String> {
    unsafe {
        for dll in [ONNX_RUNTIME_FILE_NAME, PROVIDERS_SHARED_NAME] {
            let mut path: Vec<u16> = dir.join(dll).as_os_str().encode_wide().collect();
            path.push(0);
            let _ = LoadLibraryExW(
                PCWSTR::from_raw(path.as_ptr()),
                None,
                LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32,
            )
            .map_err(|e| format!("preload {dll} failed: {e}"))?;
        }
    }
    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn prepare_onnxruntime_dll_dir(_dir: &Path) -> Result<(), String> {
    Ok(())
}
