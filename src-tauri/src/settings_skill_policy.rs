// Shared live skill gates; plugin ownership is checked on every use.
pub fn chat_native_tools_enabled(chat_tools: &ChatToolsConfig) -> bool {
    chat_tools.native_tools.any_enabled()
}

pub fn is_skill_enabled(chat_tools: &ChatToolsConfig, skill_id: &str) -> bool {
    let skill_id = skill_id.trim();
    if skill_id.is_empty() {
        return false;
    }
    !chat_tools
        .disabled_skill_ids
        .iter()
        .any(|disabled| disabled == skill_id)
}

pub fn skill_globally_available(chat_tools: &ChatToolsConfig, skill_id: &str) -> bool {
    if !crate::plugins::plugin_skill_available(skill_id) {
        return false;
    }
    is_skill_enabled(chat_tools, skill_id)
}

pub fn skill_global_unavailable_error(
    chat_tools: &ChatToolsConfig,
    skill_id: &str,
    skill_name: &str,
) -> Option<String> {
    if !crate::plugins::plugin_skill_available(skill_id) {
        if let Some(plugin_id) = crate::plugins::skill_owned_by_plugin(skill_id) {
            if crate::plugins::catalog_plugin(plugin_id).is_none() {
                return Some(format!(
                    "Operation tool «{plugin_id}» has been retired; its Skill is unavailable: {skill_name}"
                ));
            }
            return Some(format!(
                "Skill belongs to operation tool «{plugin_id}» — enable it in 设置 → 电脑操控: {skill_name}"
            ));
        }
        return Some(format!("Skill is unavailable: {skill_name}"));
    }
    if !is_skill_enabled(chat_tools, skill_id) {
        return Some(format!("Skill is disabled in Settings: {skill_name}"));
    }
    None
}
