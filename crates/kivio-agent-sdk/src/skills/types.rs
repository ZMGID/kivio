use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SkillFileKind {
    SkillMd,
    Reference,
    Script,
    Asset,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillFileEntry {
    pub relative_path: String,
    pub kind: SkillFileKind,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillMeta {
    pub id: String,
    pub name: String,
    pub description: String,
    pub source: String,
    pub path: Option<String>,
    pub recommended_tools: Vec<String>,
    #[serde(default)]
    pub disable_model_invocation: bool,
    #[serde(default)]
    pub files: Vec<SkillFileEntry>,
    /// Explicit slash triggers (e.g. `/commit`). Normalized to a leading `/` and
    /// lowercased at parse time. Empty ⇒ default `/{id}` / `/{slug(name)}`.
    #[serde(default)]
    pub triggers: Vec<String>,
    /// Free-form hint shown next to the slash command (e.g. `<message>`).
    #[serde(default)]
    pub argument_hint: Option<String>,
    /// Declared positional argument names for `$ARG_NAME` substitution.
    #[serde(default)]
    pub arguments: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct SkillRecord {
    pub meta: SkillMeta,
    pub location: PathBuf,
    pub base_dir: PathBuf,
    pub body: String,
}

impl SkillRecord {
    /// Resolve an existing resource inside this skill; this grants no read or execution access.
    pub fn resolve_resource(&self, relative: &Path) -> Result<PathBuf, String> {
        if relative.is_absolute()
            || relative.components().any(|part| {
                matches!(
                    part,
                    Component::Prefix(_) | Component::RootDir | Component::ParentDir
                )
            })
        {
            return Err("Skill resource path must stay relative to the skill directory".into());
        }
        let base = self
            .base_dir
            .canonicalize()
            .map_err(|error| format!("Resolve skill directory failed: {error}"))?;
        let resource = base
            .join(relative)
            .canonicalize()
            .map_err(|error| format!("Resolve skill resource failed: {error}"))?;
        if !resource.starts_with(&base) {
            return Err("Skill resource path escapes the skill directory".into());
        }
        Ok(resource)
    }
}

#[derive(Debug, Clone)]
pub struct SkillSource {
    pub path: PathBuf,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillDetail {
    #[serde(flatten)]
    pub meta: SkillMeta,
    pub body: String,
}

#[derive(Debug, Clone, Default)]
pub struct SkillRegistry {
    pub scan_failed: bool,
    pub records: Vec<SkillRecord>,
    pub warnings: Vec<String>,
}

impl SkillRegistry {
    /// Hydrate only explicitly selected instructions; the cached catalog stays metadata-only.
    pub fn read_selected(&self, ids: &[String]) -> Result<Self, String> {
        self.read_selected_with(ids, super::read_skill_record)
    }

    pub fn read_selected_with(
        &self,
        ids: &[String],
        reader: impl Fn(&SkillRecord) -> Result<SkillRecord, String>,
    ) -> Result<Self, String> {
        let mut loaded = self.clone();
        for id in ids {
            if let Some(record) = self.find(id) {
                let fresh = reader(record)?;
                if let Some(target) = loaded
                    .records
                    .iter_mut()
                    .find(|item| item.meta.id == record.meta.id)
                {
                    *target = fresh;
                }
            }
        }
        Ok(loaded)
    }

    pub fn find(&self, id_or_name: &str) -> Option<&SkillRecord> {
        let needle = id_or_name.trim();
        if needle.is_empty() {
            return None;
        }
        let slug = slugify(needle);
        self.records.iter().find(|record| {
            record.meta.id == needle
                || record.meta.id == slug
                || record.meta.name == needle
                || slugify(&record.meta.name) == slug
        })
    }

    pub fn metas(&self) -> Vec<SkillMeta> {
        self.records
            .iter()
            .map(|record| record.meta.clone())
            .collect()
    }

    /// Match a leading slash word (e.g. `/commit`) against skill triggers.
    /// A skill matches when `first_word` equals one of its explicit `triggers`
    /// or its default `/{id}` / `/{slug(name)}` trigger (exact match only — no
    /// prefix matching, to avoid shadowing built-in slash commands).
    pub fn find_by_trigger(&self, first_word: &str) -> Option<&SkillRecord> {
        let needle = normalize_trigger(first_word);
        if needle.len() <= 1 {
            return None;
        }
        self.records
            .iter()
            .find(|record| record_triggers(&record.meta).iter().any(|t| t == &needle))
    }
}

/// Normalize a slash trigger: trim, ensure a single leading `/`, lowercase.
pub fn normalize_trigger(value: &str) -> String {
    let trimmed = value.trim().trim_start_matches('/').trim();
    if trimmed.is_empty() {
        return String::new();
    }
    format!("/{}", trimmed.to_ascii_lowercase())
}

/// The full set of slash triggers a skill answers to: explicit `triggers` plus
/// the default `/{id}` and `/{slug(name)}` (deduped, normalized).
pub fn record_triggers(meta: &SkillMeta) -> Vec<String> {
    let mut out: Vec<String> = meta
        .triggers
        .iter()
        .map(|t| normalize_trigger(t))
        .filter(|t| t.len() > 1)
        .collect();
    for default in [
        normalize_trigger(&meta.id),
        normalize_trigger(&slugify(&meta.name)),
    ] {
        if default.len() > 1 && !out.contains(&default) {
            out.push(default);
        }
    }
    out
}

pub fn slugify(value: &str) -> String {
    let mut out = String::new();
    for c in value.chars() {
        // Keep unicode alphanumerics (CJK, Cyrillic, …), not just ASCII —
        // otherwise a Chinese-only skill name slugifies to empty and falls
        // through to the fallback below.
        if c.is_alphanumeric() {
            out.extend(c.to_lowercase());
        } else if c == '-' || c == '_' || c.is_whitespace() {
            if !out.ends_with('-') {
                out.push('-');
            }
        }
    }
    let out = out.trim_matches('-').to_string();
    if out.is_empty() {
        // Deterministic, NOT random: a skill id must be stable across scans.
        // A random uuid here made non-alphanumeric-named skills reappear after
        // being toggled off — disabled_skill_ids stored an id that changed on
        // the next scan, so the match was lost.
        format!("skill-{:x}", fnv1a(value))
    } else {
        out
    }
}

/// FNV-1a — a tiny deterministic hash (stdlib-only, stable forever) used to
/// derive a fallback skill id from names with no alphanumeric chars at all.
fn fnv1a(value: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in value.bytes() {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

pub fn parse_bool(value: Option<&str>) -> bool {
    matches!(
        value
            .map(str::trim)
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str(),
        "true" | "1" | "yes"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn record_with(id: &str, name: &str, triggers: Vec<String>) -> SkillRecord {
        SkillRecord {
            meta: SkillMeta {
                id: id.to_string(),
                name: name.to_string(),
                description: "desc".to_string(),
                source: "user".to_string(),
                path: None,
                recommended_tools: vec![],
                disable_model_invocation: false,
                files: vec![],
                triggers,
                argument_hint: None,
                arguments: vec![],
            },
            location: PathBuf::from(format!("/skills/{id}/SKILL.md")),
            base_dir: PathBuf::from(format!("/skills/{id}")),
            body: String::new(),
        }
    }

    fn registry(records: Vec<SkillRecord>) -> SkillRegistry {
        SkillRegistry {
            records,
            warnings: vec![],
            scan_failed: false,
        }
    }

    #[test]
    fn slugify_is_stable_for_non_ascii_names() {
        // The bug: a Chinese-only skill name slugified to empty -> random uuid
        // each scan -> disabled_skill_ids never matched on reload.
        assert_eq!(slugify("问题步骤拆解"), "问题步骤拆解");
        assert_eq!(slugify("问题步骤拆解"), slugify("问题步骤拆解"));
        // ASCII behavior unchanged.
        assert_eq!(slugify("Git Helper"), "git-helper");
        // No-alphanumeric names still get a stable (deterministic) fallback.
        assert_eq!(slugify("🎉🎉"), slugify("🎉🎉"));
        assert!(slugify("🎉🎉").starts_with("skill-"));
    }

    #[test]
    fn find_by_trigger_matches_explicit() {
        let reg = registry(vec![record_with(
            "git-helper",
            "Git Helper",
            vec!["/commit".to_string()],
        )]);
        let found = reg.find_by_trigger("/commit").expect("explicit trigger");
        assert_eq!(found.meta.id, "git-helper");
        // case-insensitive
        assert!(reg.find_by_trigger("/COMMIT").is_some());
    }

    #[test]
    fn find_by_trigger_default_is_slash_id() {
        let reg = registry(vec![record_with("commit", "Commit Skill", vec![])]);
        assert!(reg.find_by_trigger("/commit").is_some());
        // default also matches the slugified name
        let reg2 = registry(vec![record_with("xyz", "Commit Skill", vec![])]);
        assert!(reg2.find_by_trigger("/commit-skill").is_some());
    }

    #[test]
    fn find_by_trigger_requires_leading_slash() {
        let reg = registry(vec![record_with("commit", "Commit", vec![])]);
        assert!(reg.find_by_trigger("commit").is_some()); // normalized adds slash
        assert!(reg.find_by_trigger("/other").is_none());
        // exact only — no prefix matching
        assert!(reg.find_by_trigger("/comm").is_none());
    }
}
