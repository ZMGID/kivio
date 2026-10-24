#[path = "../../../src-tauri/src/skills/types.rs"]
pub mod types;
#[path = "../../../src-tauri/src/skills/runtime.rs"]
pub mod runtime;
#[path = "../../../src-tauri/src/skills/catalog.rs"]
pub mod catalog;
#[path = "../../../src-tauri/src/skills/parse.rs"]
pub mod parse;
#[path = "../../../src-tauri/src/skills/reader.rs"]
pub mod reader;
pub use types::*; pub use runtime::*; pub use catalog::*; pub use reader::read_skill_record;

#[path = "../../../src-tauri/src/skills/discover.rs"]
pub mod discover;
pub use discover::build_registry_with_plugins_at;

#[path = "../../../src-tauri/src/skills/package_install.rs"]
pub mod package_install;
pub use package_install::{download_skill_zip_into, download_skill_zip_into_checked, install_skill_zip_bytes};
