pub use kivio_agent_sdk::skills::SkillRunContext;
use kivio_agent_sdk::skills::{SkillAccess, SkillBodyReader, SkillRecord, SkillRegistry};
use std::sync::Arc;

struct ApplicationSkillBodyReader;

impl SkillBodyReader for ApplicationSkillBodyReader {
    fn read_body(&self, record: &SkillRecord) -> Result<String, String> {
        Ok(super::read_skill_record(record)?.body)
    }
}

pub(crate) fn for_assistant(
    registry: &Arc<SkillRegistry>,
    assistant: Option<&crate::chat::types::ChatAssistantSnapshot>,
) -> Arc<SkillRegistry> {
    let Some(assistant) = assistant.filter(|a| a.restricts_tools()) else {
        return Arc::clone(registry);
    };
    Arc::new(SkillRegistry {
        records: registry
            .records
            .iter()
            .filter(|r| assistant.skill_ids.contains(&r.meta.id))
            .cloned()
            .collect(),
        warnings: registry.warnings.clone(),
        scan_failed: registry.scan_failed,
    })
}

pub fn for_run(registry: Arc<SkillRegistry>) -> SkillRunContext {
    SkillRunContext::new(registry, SkillAccess::default())
        .with_body_reader(Arc::new(ApplicationSkillBodyReader))
}

pub fn read_selected(registry: &SkillRegistry, ids: &[String]) -> Result<SkillRegistry, String> {
    registry.read_selected_with(ids, super::read_skill_record)
}
