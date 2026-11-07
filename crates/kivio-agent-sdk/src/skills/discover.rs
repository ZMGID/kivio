use super::{parse_skill_record, SkillRecord, SkillRegistry, SkillSource};
use std::{fs, path::Path};

const MAX_SCAN_DEPTH: usize = 6;
const SKIP_DIR_NAMES: &[&str] = &[".git", "node_modules", ".svn", ".hg"];

fn read_raw(path: &Path, _source: &str) -> Result<String, String> {
    fs::read_to_string(path)
        .map_err(|error| format!("Read skill {} failed: {error}", path.display()))
}

impl SkillRegistry {
    /// Sources are ordered by priority; the first occurrence of an id wins.
    pub fn discover(sources: &[SkillSource]) -> Self {
        Self::discover_with_reader(sources, read_raw)
    }

    pub fn discover_with_reader(
        sources: &[SkillSource],
        reader: impl Fn(&Path, &str) -> Result<String, String>,
    ) -> Self {
        let mut registry = SkillRegistry::default();
        for root in sources {
            if let Err(err) =
                collect_skill_files(&root.path, 0, &mut registry, &root.source, &reader)
            {
                registry.scan_failed = true;
                registry
                    .warnings
                    .push(format!("Scan {} failed: {err}", root.path.display()));
            }
        }
        dedup_records(&mut registry.records, &mut registry.warnings);
        registry
            .records
            .sort_by(|a, b| a.meta.name.to_lowercase().cmp(&b.meta.name.to_lowercase()));
        registry
    }

    /// Replace the catalog only after a complete successful scan.
    pub fn reload(&mut self, sources: &[SkillSource]) -> Result<(), String> {
        self.reload_with_reader(sources, read_raw)
    }

    pub fn reload_with_reader(
        &mut self,
        sources: &[SkillSource],
        reader: impl Fn(&Path, &str) -> Result<String, String>,
    ) -> Result<(), String> {
        let next = Self::discover_with_reader(sources, reader);
        if next.scan_failed {
            return Err(next.warnings.join("\n"));
        }
        *self = next;
        Ok(())
    }
}

fn dedup_records(records: &mut Vec<super::types::SkillRecord>, warnings: &mut Vec<String>) {
    let mut seen = std::collections::HashMap::<String, usize>::new();
    let mut out: Vec<super::types::SkillRecord> = Vec::new();
    for record in records.drain(..) {
        let key = record.meta.id.clone();
        if let Some(&index) = seen.get(&key) {
            // 同 ID 按扫描顺序取第一份；跨来源允许覆盖，同来源重复才报告冲突。
            if out[index].meta.source == record.meta.source {
                warnings.push(format!(
                    "Skill {} shadowed duplicate id {}",
                    out[index].meta.name, record.meta.name
                ));
            }
            continue;
        }
        seen.insert(key, out.len());
        out.push(record);
    }
    *records = out;
}

fn collect_skill_files(
    root: &Path,
    depth: usize,
    registry: &mut SkillRegistry,
    source: &str,
    reader: &impl Fn(&Path, &str) -> Result<String, String>,
) -> Result<(), String> {
    if depth > MAX_SCAN_DEPTH || !root.is_dir() {
        return Ok(());
    }

    let skill_md = root.join("SKILL.md");
    if skill_md.is_file() {
        match reader(&skill_md, source).and_then(|raw| {
            parse_skill_record(&skill_md, &raw, source, Vec::new(), &mut Vec::new())
        }) {
            Ok(mut record) => {
                record.body.clear();
                registry.records.push(record);
            }
            Err(err) => {
                registry.scan_failed = true;
                registry
                    .warnings
                    .push(format!("Parse skill {} failed: {err}", skill_md.display()));
            }
        }
        return Ok(());
    }

    if depth == MAX_SCAN_DEPTH {
        return Ok(());
    }

    let mut paths = fs::read_dir(root)
        .map_err(|err| format!("read skills dir failed: {err}"))?
        .map(|entry| {
            entry
                .map(|entry| entry.path())
                .map_err(|err| err.to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    paths.sort();
    for path in paths {
        if !path.is_dir() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if SKIP_DIR_NAMES.contains(&name) {
            continue;
        }
        collect_skill_files(&path, depth + 1, registry, source, reader)?;
    }
    Ok(())
}

/// Read the current instructions while retaining the catalog's frozen identity and metadata.
pub fn read_skill_record(record: &SkillRecord) -> Result<SkillRecord, String> {
    read_skill_record_with(record, &read_raw(&record.location, &record.meta.source)?)
}

/// Apply host-provided markdown with the same frozen-catalog semantics as filesystem reads.
pub fn read_skill_record_with(record: &SkillRecord, raw: &str) -> Result<SkillRecord, String> {
    let mut loaded = parse_skill_record(
        &record.location,
        raw,
        &record.meta.source,
        Vec::new(),
        &mut Vec::new(),
    )?;
    loaded.meta = record.meta.clone();
    loaded.meta.files.clear();
    Ok(loaded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    type SkillScanRoot = SkillSource;

    fn build_registry_from_roots(roots: Vec<SkillSource>) -> SkillRegistry {
        SkillRegistry::discover(&roots)
    }

    fn write_skill(dir: &Path, id: &str) {
        fs::create_dir_all(dir).unwrap();
        fs::write(
            dir.join("SKILL.md"),
            format!("---\nname: {id}\ndescription: {id}\n---\n"),
        )
        .unwrap();
    }

    fn temp_skill_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kivio-skill-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(dir.join("scripts")).unwrap();
        fs::write(
            dir.join("SKILL.md"),
            r#"---
name: skill-test
description: Test skill.
---

# Skill body
"#,
        )
        .unwrap();
        fs::write(dir.join("scripts").join("run.sh"), "echo ok").unwrap();
        dir
    }

    #[test]
    fn discovery_does_not_index_skill_resources() {
        let dir = temp_skill_dir();
        fs::create_dir_all(dir.join("artifacts/nested")).unwrap();
        fs::write(dir.join("artifacts/nested/output.txt"), "generated").unwrap();
        write_skill(&dir.join("artifacts/nested/fake-skill"), "nested-artifact");
        let registry = build_registry_from_roots(vec![SkillScanRoot {
            path: dir.clone(),
            source: "user".into(),
        }]);
        fs::remove_dir_all(dir).unwrap();
        assert_eq!(registry.records.len(), 1);
        assert!(
            registry.records[0].meta.files.is_empty(),
            "discovery must not enumerate skill resources"
        );
        assert!(registry.records[0].body.is_empty());
    }

    #[test]
    fn same_id_prefers_project_over_agents() {
        let base =
            std::env::temp_dir().join(format!("kivio-agents-dedup-{}", uuid::Uuid::new_v4()));
        let project_root = base.join("project").join(".agents").join("skills");
        let agents_root = base.join("home").join(".agents").join("skills");
        write_skill(&project_root.join("shared"), "shared");
        write_skill(&agents_root.join("shared"), "shared");

        let registry = build_registry_from_roots(vec![
            SkillScanRoot {
                path: project_root,
                source: "project".into(),
            },
            SkillScanRoot {
                path: agents_root,
                source: "agents".into(),
            },
        ]);
        assert_eq!(registry.records.len(), 1);
        assert_eq!(registry.records[0].meta.source, "project");
        assert!(
            registry.warnings.is_empty(),
            "cross-source overlay is expected, not a user warning: {:?}",
            registry.warnings
        );

        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn same_source_duplicate_id_warns() {
        let base =
            std::env::temp_dir().join(format!("kivio-agents-samedup-{}", uuid::Uuid::new_v4()));
        let one = base.join("a");
        let two = base.join("b");
        write_skill(&one.join("shared"), "shared");
        write_skill(&two.join("shared"), "shared");

        let registry = build_registry_from_roots(vec![
            SkillScanRoot {
                path: one,
                source: "user".into(),
            },
            SkillScanRoot {
                path: two,
                source: "user".into(),
            },
        ]);
        assert_eq!(registry.records.len(), 1);
        assert_eq!(registry.warnings.len(), 1);
        assert!(
            registry.warnings[0].contains("shadowed duplicate id"),
            "{:?}",
            registry.warnings
        );

        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn malformed_skills_are_reported_while_valid_catalog_survives() {
        let base = temp_skill_dir();
        fs::remove_file(base.join("SKILL.md")).unwrap();
        write_skill(&base.join("valid"), "valid");
        fs::create_dir_all(base.join("invalid")).unwrap();
        fs::write(base.join("invalid/SKILL.md"), "---\nname: invalid\n---\n").unwrap();
        let registry = build_registry_from_roots(vec![SkillSource {
            path: base.clone(),
            source: "test".into(),
        }]);
        assert!(registry.scan_failed);
        assert!(registry.find("valid").is_some());
        assert!(registry.find("invalid").is_none());
        assert!(registry
            .warnings
            .iter()
            .any(|warning| warning.contains("description")));
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn scan_has_deterministic_sibling_priority_depth_limit_and_directory_exclusions() {
        let base = temp_skill_dir();
        fs::remove_file(base.join("SKILL.md")).unwrap();
        write_skill(&base.join("b"), "duplicate");
        write_skill(&base.join("a"), "duplicate");
        write_skill(&base.join("one/two/three/four/five/six"), "at-limit");
        write_skill(
            &base.join("one/two/three/four/five/other/seven"),
            "too-deep",
        );
        write_skill(&base.join("node_modules/ignored"), "ignored");
        let registry = build_registry_from_roots(vec![SkillSource {
            path: base.clone(),
            source: "test".into(),
        }]);
        assert_eq!(
            registry.find("duplicate").unwrap().location,
            base.join("a/SKILL.md")
        );
        assert!(registry.find("at-limit").is_some());
        assert!(registry.find("too-deep").is_none());
        assert!(registry.find("ignored").is_none());
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn body_reads_preserve_frozen_metadata_and_reload_failure_preserves_catalog() {
        let base = temp_skill_dir();
        let sources = [SkillSource {
            path: base.clone(),
            source: "test".into(),
        }];
        let mut registry = SkillRegistry::discover(&sources);
        let frozen = registry.clone();
        fs::write(
            base.join("SKILL.md"),
            "---\nname: renamed\ndescription: changed\narguments: [new]\n---\nUPDATED",
        )
        .unwrap();
        let loaded = registry.read_selected(&["skill-test".into()]).unwrap();
        let record = loaded.find("skill-test").unwrap();
        assert_eq!(record.meta.name, "skill-test");
        assert_eq!(record.meta.description, "Test skill.");
        assert!(record.meta.arguments.is_empty());
        assert_eq!(record.body, "UPDATED");
        assert!(registry.find("skill-test").unwrap().body.is_empty());
        registry.reload(&sources).unwrap();
        assert!(registry.find("renamed").is_some());
        assert!(frozen.find("renamed").is_none());
        fs::write(base.join("SKILL.md"), "---\nname: invalid\n---\n").unwrap();
        assert!(registry
            .reload(&sources)
            .unwrap_err()
            .contains("description"));
        assert!(registry.find("renamed").is_some());
        fs::remove_file(base.join("SKILL.md")).unwrap();
        assert!(read_skill_record(registry.find("renamed").unwrap()).is_err());
        registry.reload(&sources).unwrap();
        assert!(registry.records.is_empty());
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn source_reader_is_used_for_discovery_and_selected_hydration() {
        let base = temp_skill_dir();
        fs::write(base.join("SKILL.md"), "host-specific format").unwrap();
        let registry = SkillRegistry::discover_with_reader(
            &[SkillSource {
                path: base.clone(),
                source: "custom".into(),
            }],
            |path, source| {
                assert_eq!(source, "custom");
                assert_eq!(path, base.join("SKILL.md"));
                Ok("---\nname: custom\ndescription: transformed\n---\nInitial".into())
            },
        );
        assert!(!registry.scan_failed);
        let selected = registry
            .read_selected_with(&["custom".into()], |record| {
                read_skill_record_with(
                    record,
                    "---\nname: changed\ndescription: changed\n---\nFresh host body",
                )
            })
            .unwrap();
        assert_eq!(selected.find("custom").unwrap().body, "Fresh host body");
        assert_eq!(selected.find("custom").unwrap().meta.name, "custom");
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn resource_resolution_accepts_contained_paths_and_rejects_escape_or_missing() {
        let base = temp_skill_dir();
        let registry = SkillRegistry::discover(&[SkillSource {
            path: base.clone(),
            source: "test".into(),
        }]);
        let record = &registry.records[0];
        assert_eq!(
            record
                .resolve_resource(Path::new("scripts/run.sh"))
                .unwrap(),
            base.join("scripts/run.sh").canonicalize().unwrap()
        );
        assert!(record.resolve_resource(Path::new("../SKILL.md")).is_err());
        assert!(record.resolve_resource(&base.join("SKILL.md")).is_err());
        assert!(record.resolve_resource(Path::new("missing")).is_err());
        #[cfg(windows)]
        assert!(record.resolve_resource(Path::new("C:SKILL.md")).is_err());
        fs::remove_dir_all(base).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn resource_resolution_rejects_symlinks_outside_skill_directory() {
        let base = temp_skill_dir();
        let outside = temp_skill_dir();
        std::os::unix::fs::symlink(outside.join("SKILL.md"), base.join("escape")).unwrap();
        let registry = SkillRegistry::discover(&[SkillSource {
            path: base.clone(),
            source: "test".into(),
        }]);
        assert!(registry.records[0]
            .resolve_resource(Path::new("escape"))
            .is_err());
        fs::remove_dir_all(base).unwrap();
        fs::remove_dir_all(outside).unwrap();
    }
}
