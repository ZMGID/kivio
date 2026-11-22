pub use kivio_agent_sdk::skills::SkillRunContext;
use kivio_agent_sdk::skills::{SkillAccess, SkillBodyReader, SkillRecord, SkillRegistry};
use std::sync::Arc;

struct ApplicationSkillBodyReader;

impl SkillBodyReader for ApplicationSkillBodyReader {
    fn read_body(&self, record: &SkillRecord) -> Result<String, String> {
        Ok(super::read_skill_record(record)?.body)
    }
}

pub fn for_run(registry: Arc<SkillRegistry>) -> SkillRunContext {
    SkillRunContext::new(registry, SkillAccess::default())
        .with_body_reader(Arc::new(ApplicationSkillBodyReader))
}

pub fn read_selected(registry: &SkillRegistry, ids: &[String]) -> Result<SkillRegistry, String> {
    registry.read_selected_with(ids, super::read_skill_record)
}
