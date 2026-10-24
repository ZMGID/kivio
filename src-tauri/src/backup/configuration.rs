use serde::{Serialize, Deserialize};
use std::collections::HashSet;
use crate::chat::{conversation_data::ChatAssistant, group_catalog::{ChatSet, validate_set_id, normalize_set_name, validate_assistant_id}};
/// The set catalog travels with settings so restoring preferences also restores
/// their named personas and all assistant definitions.
#[derive(Serialize, Deserialize)]
pub struct SetCatalogBackup {
    pub sets: Vec<ChatSet>,
    pub assistants: Vec<ChatAssistant>,
}

impl SetCatalogBackup {
    pub fn validate(&mut self) -> Result<(), String> {
        let mut ids = HashSet::new();
        for set in &mut self.sets {
            validate_set_id(&set.id)?;
            set.name = normalize_set_name(&set.name)?;
            if !ids.insert(set.id.clone()) {
                return Err("备份包含重复集".into());
            }
        }
        ids.clear();
        for assistant in &self.assistants {
            validate_assistant_id(&assistant.id)?;
            if !ids.insert(assistant.id.clone()) {
                return Err("备份包含重复助手".into());
            }
        }
        Ok(())
    }
}
