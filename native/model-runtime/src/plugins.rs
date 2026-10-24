#[path = "../../../src-tauri/src/plugins/package_catalog.rs"]
pub mod package_catalog;
#[path = "../../../src-tauri/src/plugins/catalog.rs"]
pub mod catalog;
#[path = "../../../src-tauri/src/plugins/state.rs"]
pub mod state;
#[path = "../../../src-tauri/src/plugins/package_content.rs"]
pub mod package_content;
pub use state::*; pub use catalog::*;
pub mod packages {
 pub use super::package_catalog::{Package, Resolved};
 pub fn discover_active() -> Result<Vec<Resolved>, String> { super::package_catalog::load_active(&root()?, true) }
 pub fn active() -> Vec<Resolved> { root().ok().and_then(|root| super::package_catalog::load_active(&root, false).ok()).unwrap_or_default() }
 fn root() -> Result<std::path::PathBuf, String> { super::plugins_root().map(|p| p.join("packages")).ok_or("Application data directory unavailable".into()) }
 pub fn owner_enabled(id: &str) -> bool { root().is_ok_and(|root| super::package_content::owner_enabled(&root,id)) }
 pub fn expand_skill_body(id: &str, location: &std::path::Path, body: &str) -> Result<String,String> { super::package_content::expand_skill_body(&root()?,id,location,body) }
 pub use super::package_content::component_markdown;
}

#[path = "../../../src-tauri/src/plugins/skill_files.rs"]
pub mod skill_files;
pub use skill_files::enabled_skill_roots;

#[path = "../../../src-tauri/src/plugins/install_core.rs"]
pub mod install_core;
pub mod native_control;
