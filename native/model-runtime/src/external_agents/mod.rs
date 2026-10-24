//! Existing CLI protocol owners assembled without the Tauri host.
#[path = "../../../../src-tauri/src/external_agents/attachments.rs"]
pub mod attachments;
#[path = "../../../../src-tauri/src/external_agents/errors.rs"]
pub mod errors;
#[path = "../../../../src-tauri/src/external_agents/process_io.rs"]
pub mod process_io;
#[path = "../../../../src-tauri/src/external_agents/slash_data.rs"]
pub mod slash;
pub mod spawn;
#[path = "../../../../src-tauri/src/external_agents/stream/mod.rs"]
pub mod stream;
#[path = "../../../../src-tauri/src/external_agents/types.rs"]
pub mod types;
#[path = "../../../../src-tauri/src/external_agents/wsl.rs"]
pub mod wsl;
pub mod registry {
    pub static AGENT_DEFS: &[super::types::RuntimeAgentDef] =
        &[super::defs::codex::CODEX_AGENT_DEF];
    pub fn get_agent_def(id: &str) -> Option<&'static super::types::RuntimeAgentDef> {
        AGENT_DEFS.iter().find(|d| d.id == id)
    }
}
pub mod defs;
pub mod native;
pub mod session;

#[path = "../../../../src-tauri/src/external_agents/import_record.rs"]
pub mod import;

#[path = "../../../../src-tauri/src/external_agents/management_catalog.rs"]
pub mod management_catalog;
#[path = "../../../../src-tauri/src/external_agents/account.rs"]
pub mod account;
pub mod management;

#[path = "../../../../src-tauri/src/external_agents/installer_core.rs"]
pub mod installer;
