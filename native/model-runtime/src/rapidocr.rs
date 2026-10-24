//! Lightweight client for the separately packaged OCR process.
use std::{path::{Path, PathBuf}, sync::Arc, process::Stdio};
use serde::{Serialize, Deserialize};
use tokio::sync::Mutex;
use crate::offline_models::OfflineModelManager;
use crate::proc::NoConsoleWindow;
pub use crate::offline_models::OcrModelTier as ModelTier;
include!("../../../src-tauri/src/rapidocr/protocol.rs");
include!("../../../src-tauri/src/rapidocr/types.rs");
include!("../../../src-tauri/src/rapidocr/client.rs");

include!("../../../src-tauri/src/rapidocr/geometry_math.rs");
