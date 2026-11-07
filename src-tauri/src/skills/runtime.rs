pub use kivio_agent_sdk::skills::SkillRunContext;
use kivio_agent_sdk::skills::{SkillAccess, SkillBodyReader, SkillRecord, SkillRegistry};
use std::sync::Arc;

struct ApplicationSkillBodyReader;

impl SkillBodyReader for ApplicationSkillBodyReader {
    fn read_body(&self, record: &SkillRecord) -> Result<String, String> {
        Ok(super::read_skill_record(record)?.body)
    }
}

pub fn for_run(
    registry: Arc<SkillRegistry>,
    assistant: Option<&crate::chat::types::ChatAssistantSnapshot>,
) -> SkillRunContext {
    let access = SkillAccess {
        allowed_ids: assistant
            .filter(|assistant| !crate::chat::voice_orchestrator::is_brain_snapshot(assistant))
            .map(|assistant| assistant.skill_ids.clone()),
        ..Default::default()
    };
    SkillRunContext::new(registry, access).with_body_reader(Arc::new(ApplicationSkillBodyReader))
}

pub fn read_selected(registry: &SkillRegistry, ids: &[String]) -> Result<SkillRegistry, String> {
    registry.read_selected_with(ids, super::read_skill_record)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_context_freezes_catalog_and_assistant_authority() {
        let registry = Arc::new(SkillRegistry::default());
        let mut assistant: crate::chat::types::ChatAssistantSnapshot = serde_json::from_value(
            serde_json::json!({"id": "assistant", "name": "Assistant", "skill_ids": ["allowed"]}),
        )
        .unwrap();
        let context = for_run(registry.clone(), Some(&assistant));
        assistant.skill_ids.clear();
        assert!(context.skill_id_allowed("allowed"));
        assert!(!context.skill_id_allowed("other"));
        assert!(std::ptr::eq(context.registry(), registry.as_ref()));
        assert!(!for_run(registry.clone(), Some(&assistant)).skill_id_allowed("allowed"));
        assert!(for_run(registry.clone(), None).skill_id_allowed("other"));
        assistant.id = "asst_voice_orchestrator".into();
        assert!(for_run(registry, Some(&assistant)).skill_id_allowed("other"));
    }
}
