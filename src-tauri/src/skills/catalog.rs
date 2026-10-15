use super::types::SkillRegistry;

pub fn format_catalog(
    registry: &SkillRegistry,
    explicit_skill_id: Option<&str>,
    skill_enabled: impl Fn(&str) -> bool,
) -> String {
    let explicit_skill_id = explicit_skill_id.filter(|id| !id.trim().is_empty());
    let mut skills: Vec<_> = registry
        .records
        .iter()
        .filter(|record| skill_enabled(&record.meta.id))
        .filter(|record| {
            if !record.meta.disable_model_invocation {
                return true;
            }
            if let Some(explicit) = explicit_skill_id {
                return record.meta.id == explicit
                    || record.meta.name == explicit
                    || super::types::slugify(explicit) == record.meta.id;
            }
            false
        })
        .collect();

    if skills.is_empty() {
        return String::new();
    }

    skills.sort_by(|a, b| a.meta.name.cmp(&b.meta.name));

    let mut out = String::new();
    out.push_str("<available_skills>\n");
    for record in skills {
        out.push_str("  <skill>\n");
        out.push_str(&format!(
            "    <name>{}</name>\n",
            xml_escape(&record.meta.name)
        ));
        out.push_str(&format!(
            "    <description>{}</description>\n",
            xml_escape(&record.meta.description)
        ));
        out.push_str(&format!(
            "    <location>{}</location>\n",
            xml_escape(&record.location.display().to_string())
        ));
        out.push_str("  </skill>\n");
    }
    out.push_str("</available_skills>");
    out
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skills::types::{SkillMeta, SkillRecord, SkillRegistry};
    use std::path::PathBuf;

    fn sample_record(id: &str, name: &str, disable_model_invocation: bool) -> SkillRecord {
        SkillRecord {
            meta: SkillMeta {
                id: id.to_string(),
                name: name.to_string(),
                description: format!("{name} skill"),
                source: "user".to_string(),
                path: None,
                recommended_tools: vec![],
                disable_model_invocation,
                files: vec![],
                triggers: vec![],
                argument_hint: None,
                arguments: vec![],
            },
            location: PathBuf::from(format!("/skills/{id}/SKILL.md")),
            base_dir: PathBuf::from(format!("/skills/{id}")),
            body: String::new(),
        }
    }

    #[test]
    fn catalog_hides_disable_model_invocation_unless_explicit() {
        let registry = SkillRegistry {
            records: vec![
                sample_record("auto", "auto", false),
                sample_record("manual", "manual", true),
            ],
            warnings: vec![],
            scan_failed: false,
        };

        let catalog = format_catalog(&registry, None, |_| true);
        assert!(catalog.contains("auto"));
        assert!(!catalog.contains("manual"));
        assert!(catalog.contains("<location>/skills/auto/SKILL.md</location>"));

        let explicit = format_catalog(&registry, Some("manual"), |_| true);
        assert!(explicit.contains("manual"));
        assert!(explicit.contains("<location>/skills/manual/SKILL.md</location>"));
    }

    #[test]
    fn catalog_contains_metadata_without_activation_policy() {
        let registry = SkillRegistry {
            records: vec![sample_record("auto", "auto", false)],
            warnings: vec![],
            scan_failed: false,
        };

        let catalog = format_catalog(&registry, None, |_| true);
        assert!(!catalog.contains("call the skill tool"));
        assert!(catalog.starts_with("<available_skills>"));
        assert!(catalog.contains("<location>/skills/auto/SKILL.md</location>"));
    }

    #[test]
    fn catalog_respects_skill_enabled_filter() {
        let registry = SkillRegistry {
            records: vec![
                sample_record("auto", "auto", false),
                sample_record("off", "off", false),
            ],
            warnings: vec![],
            scan_failed: false,
        };

        let catalog = format_catalog(&registry, None, |id| id != "off");
        assert!(catalog.contains("auto"));
        assert!(!catalog.contains("off"));
    }
}
