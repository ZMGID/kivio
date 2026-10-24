use std::{fs, path::Path};
use super::{types::SkillRecord, parse::parse_skill_record};

pub(super) fn load_skill_at(skill_md_path: &Path, source: &str) -> Result<super::types::SkillRecord, String> {
    let raw = if source == "plugin" {
        crate::plugins::packages::component_markdown(skill_md_path)?
    } else {
        fs::read_to_string(skill_md_path)
            .map_err(|err| format!("Read skill {} failed: {err}", skill_md_path.display()))?
    };
    let mut warnings = Vec::new();
    parse_skill_record(skill_md_path, &raw, source, Vec::new(), &mut warnings)
}

/// Load only the known skill file. The session's identity and metadata stay stable
/// until /reload; instruction edits take effect on the next explicit use.
pub fn read_skill_record(record: &SkillRecord) -> Result<SkillRecord, String> {
    let mut loaded = load_skill_at(&record.location, &record.meta.source)?;
    loaded.meta = record.meta.clone();
    loaded.meta.files.clear();
    if record.meta.source == "plugin" && record.meta.id.starts_with("pkg-") {
        loaded.body = crate::plugins::packages::expand_skill_body(
            &record.meta.id,
            &record.location,
            &loaded.body,
        )?;
    }
    Ok(loaded)
}
