//! Native host for the original portable set owner. Its file validation,
//! attachment/CLI identities and rollback rules remain shared with Desktop.
#[cfg(test)]
use crate::file_io::fail_atomic_replace_for_current_test;
use crate::{
    conversation_data::{
        ChatAssistant, ChatAssistantIndex, Conversation, ConversationListItem, ConversationPin,
    },
    conversation_files::{ConversationStore, read_conversation_file, validate_conversation_id},
    file_io::atomic_write,
    group_catalog::{self as sets, ChatSet, ChatSetIndex},
};
pub(crate) use crate::backup_configuration::SetCatalogBackup;
use serde::Deserialize;
use std::{
    collections::{HashMap, HashSet},
    fs,
    io::{ErrorKind, Write},
    path::{Path, PathBuf},
};

mod index {
    pub(super) use crate::conversation_files::{
        conversation_file_ids_in_dir, load_index_or_scan_in_dir,
    };
    // Native ConversationStore has no index cache: each operation reads fresh disk state.
    pub(super) fn forget_index_cache(_: &std::path::Path) {}
}
fn catalog_mutation_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|p| p.into_inner())
}
#[path = "../../../src-tauri/src/chat/storage/set_backup.rs"]
mod files;

pub fn export(root: &Path, set_id: &str, path: &Path) -> Result<(), String> {
    ConversationStore::new(root.join("conversations"))
        .with_exclusive(|| files::export_in(root, set_id, path))
}
pub fn import(root: &Path, path: &Path) -> Result<ChatSet, String> {
    ConversationStore::new(root.join("conversations"))
        .with_exclusive(|| files::import_in(root, path))
}
