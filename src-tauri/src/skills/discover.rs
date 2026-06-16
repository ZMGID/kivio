use std::{
    fs,
    path::{Path, PathBuf},
};

use tauri::{AppHandle, Manager};

use super::{
    parse::parse_skill_record,
    types::{slugify, SkillFileEntry, SkillFileKind, SkillRegistry},
};

const MAX_SCAN_DEPTH: usize = 6;

const SKIP_DIR_NAMES: &[&str] = &[".git", "node_modules", ".svn", ".hg"];

struct SkillScanRoot {
    path: PathBuf,
    source: &'static str,
}

pub fn user_skills_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|err| format!("app_data_dir unavailable: {err}"))?
        .join("skills");
    fs::create_dir_all(&dir).map_err(|err| format!("create skills dir failed: {err}"))?;
    Ok(dir)
}

fn bundled_skills_dir(app: &AppHandle) -> Option<PathBuf> {
    app.path()
        .resource_dir()
        .ok()
        .map(|dir| dir.join("skills"))
        .filter(|dir| dir.is_dir())
}

#[allow(dead_code)]
pub fn scan_roots(app: &AppHandle, extra_paths: &[String]) -> Result<Vec<PathBuf>, String> {
    Ok(scan_root_entries(app, extra_paths)?
        .into_iter()
        .map(|entry| entry.path)
        .collect())
}

fn scan_root_entries(
    app: &AppHandle,
    extra_paths: &[String],
) -> Result<Vec<SkillScanRoot>, String> {
    let mut roots = Vec::new();
    if let Some(path) = bundled_skills_dir(app) {
        roots.push(SkillScanRoot {
            path,
            source: "builtin",
        });
    }
    roots.push(SkillScanRoot {
        path: user_skills_dir(app)?,
        source: "user",
    });
    roots.extend(extra_paths.iter().map(PathBuf::from).filter_map(|path| {
        path.is_dir().then_some(SkillScanRoot {
            path,
            source: "external",
        })
    }));
    Ok(roots)
}

pub fn build_registry(app: &AppHandle, extra_paths: &[String]) -> Result<SkillRegistry, String> {
    build_registry_inner(app, extra_paths, true)
}

pub fn build_registry_metadata(
    app: &AppHandle,
    extra_paths: &[String],
) -> Result<SkillRegistry, String> {
    build_registry_inner(app, extra_paths, false)
}

fn build_registry_inner(
    app: &AppHandle,
    extra_paths: &[String],
    include_files: bool,
) -> Result<SkillRegistry, String> {
    let mut registry = SkillRegistry::default();
    let roots = scan_root_entries(app, extra_paths)?;

    for root in roots {
        collect_skill_files(&root.path, 0, &mut registry, root.source, include_files)?;
    }

    dedup_records(&mut registry.records, &mut registry.warnings);
    registry
        .records
        .sort_by(|a, b| a.meta.name.to_lowercase().cmp(&b.meta.name.to_lowercase()));
    Ok(registry)
}

fn dedup_records(records: &mut Vec<super::types::SkillRecord>, warnings: &mut Vec<String>) {
    let mut seen = std::collections::HashMap::<String, usize>::new();
    let mut out: Vec<super::types::SkillRecord> = Vec::new();
    for record in records.drain(..) {
        let key = record.meta.id.clone();
        if let Some(index) = seen.get(&key) {
            warnings.push(format!(
                "Skill {} shadowed duplicate id {}",
                out[*index].meta.name, record.meta.name
            ));
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
    include_files: bool,
) -> Result<(), String> {
    if depth > MAX_SCAN_DEPTH || !root.is_dir() {
        return Ok(());
    }

    let skill_md = root.join("SKILL.md");
    if skill_md.is_file() {
        match load_skill_at(&skill_md, source, include_files) {
            Ok(record) => registry.records.push(record),
            Err(err) => registry
                .warnings
                .push(format!("Parse skill {} failed: {err}", skill_md.display())),
        }
        return Ok(());
    }

    if depth == MAX_SCAN_DEPTH {
        return Ok(());
    }

    let entries = fs::read_dir(root).map_err(|err| format!("read skills dir failed: {err}"))?;
    for entry in entries {
        let entry = entry.map_err(|err| err.to_string())?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if SKIP_DIR_NAMES.contains(&name) {
            continue;
        }
        collect_skill_files(&path, depth + 1, registry, source, include_files)?;
    }
    Ok(())
}

fn load_skill_at(
    skill_md_path: &Path,
    source: &str,
    include_files: bool,
) -> Result<super::types::SkillRecord, String> {
    let raw = fs::read_to_string(skill_md_path)
        .map_err(|err| format!("Read skill {} failed: {err}", skill_md_path.display()))?;
    let base_dir = skill_md_path
        .parent()
        .ok_or_else(|| "Skill path has no parent directory".to_string())?;
    let files = if include_files {
        index_skill_files(base_dir)?
    } else {
        Vec::new()
    };
    let mut warnings = Vec::new();
    parse_skill_record(skill_md_path, &raw, source, files, &mut warnings)
}

pub fn index_skill_files(base_dir: &Path) -> Result<Vec<SkillFileEntry>, String> {
    let mut files = Vec::new();
    walk_files(base_dir, base_dir, 0, &mut files)?;
    files.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    Ok(files)
}

fn walk_files(
    base_dir: &Path,
    current: &Path,
    depth: usize,
    out: &mut Vec<SkillFileEntry>,
) -> Result<(), String> {
    if depth > MAX_SCAN_DEPTH {
        return Ok(());
    }
    let entries = fs::read_dir(current).map_err(|err| err.to_string())?;
    for entry in entries {
        let entry = entry.map_err(|err| err.to_string())?;
        let path = entry.path();
        let file_name = entry.file_name();
        let name = file_name.to_string_lossy();
        if path.is_dir() {
            if SKIP_DIR_NAMES.iter().any(|skip| *skip == name.as_ref()) {
                continue;
            }
            walk_files(base_dir, &path, depth + 1, out)?;
            continue;
        }
        if !path.is_file() {
            continue;
        }
        let relative = path
            .strip_prefix(base_dir)
            .map_err(|err| err.to_string())?
            .to_string_lossy()
            .replace('\\', "/");
        if relative == "SKILL.md" {
            continue;
        }
        let size_bytes = fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0);
        out.push(SkillFileEntry {
            relative_path: relative.clone(),
            kind: classify_file(&relative),
            size_bytes,
        });
    }
    Ok(())
}

fn classify_file(relative_path: &str) -> SkillFileKind {
    let normalized = relative_path.replace('\\', "/");
    if normalized.starts_with("scripts/") {
        return SkillFileKind::Script;
    }
    if normalized.starts_with("references/") || normalized.ends_with(".md") {
        return SkillFileKind::Reference;
    }
    if normalized.starts_with("assets/") {
        return SkillFileKind::Asset;
    }
    SkillFileKind::Other
}

#[allow(dead_code)]
pub fn folder_slug_for_path(path: &Path) -> String {
    path.parent()
        .and_then(|parent| parent.file_name())
        .and_then(|name| name.to_str())
        .map(slugify)
        .unwrap_or_else(|| "skill".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

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
    fn metadata_registry_skips_bundled_file_indexing() {
        let dir = temp_skill_dir();
        let skill_md = dir.join("SKILL.md");

        let metadata_record = load_skill_at(&skill_md, "user", false).unwrap();
        assert!(metadata_record.meta.files.is_empty());

        let full_record = load_skill_at(&skill_md, "user", true).unwrap();
        assert_eq!(full_record.meta.files.len(), 1);
        assert_eq!(full_record.meta.files[0].relative_path, "scripts/run.sh");

        fs::remove_dir_all(dir).unwrap();
    }
}
