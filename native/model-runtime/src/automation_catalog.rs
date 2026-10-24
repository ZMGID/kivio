//! Original automation schemas and read-only catalog. No graph execution host.
#[path = "../../../src-tauri/src/automation/catalog.rs"]
mod catalog;
#[path = "../../../src-tauri/src/automation/types.rs"]
pub mod types;
pub use catalog::{list_in, read_one};
