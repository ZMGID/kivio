mod catalog;
mod discover;
pub mod parse;
mod runtime;
mod types;

pub use catalog::format_catalog;
pub use discover::{
    build_registry, build_registry_headless, build_registry_metadata, user_skills_dir,
    user_skills_dir_headless,
};
pub use parse::parse_skill_markdown;
pub use runtime::{
    activate_skill, extract_skill_name, lookup_skill, substitute_arguments, SkillRunCache,
};
pub use types::{
    slugify, SkillDetail, SkillImportResult, SkillListResult, SkillMeta, SkillOpenFolderResult,
    SkillReadResult, SkillRecord, SkillRegistry,
};

use std::{
    fs,
    io::{Cursor, Read},
    path::{Path, PathBuf},
};

use tauri::{AppHandle, State};
use tauri_plugin_shell::ShellExt;

use crate::state::AppState;

#[tauri::command]
pub fn chat_skills_list(
    app: AppHandle,
    state: State<'_, AppState>,
    skill_scan_paths: Option<Vec<String>>,
) -> SkillListResult {
    let paths = skill_scan_paths
        .unwrap_or_else(|| state.settings_read().chat_tools.skill_scan_paths.clone());
    match build_registry_metadata(&app, &paths) {
        Ok(registry) => {
            let settings = state.settings_read();
            let skills = registry
                .metas()
                .into_iter()
                .filter(|meta| {
                    crate::settings::skill_connector_satisfied(
                        &meta.id,
                        &settings.email_accounts,
                        crate::settings::obsidian_connector_configured(
                            &settings.obsidian_vault_path,
                        ),
                    )
                })
                .collect();
            SkillListResult {
                success: true,
                skills,
                error: if registry.warnings.is_empty() {
                    None
                } else {
                    Some(registry.warnings.join("\n"))
                },
                warnings: registry.warnings,
            }
        }
        Err(err) => SkillListResult {
            success: false,
            skills: Vec::new(),
            warnings: Vec::new(),
            error: Some(err),
        },
    }
}

#[tauri::command]
pub fn chat_skills_read(
    app: AppHandle,
    state: State<'_, AppState>,
    skill_id: String,
) -> SkillReadResult {
    let settings = state.settings_read();
    if let Some(err) = crate::settings::skill_global_unavailable_error(
        &settings.chat_tools,
        &skill_id,
        &settings.email_accounts,
        crate::settings::obsidian_connector_configured(&settings.obsidian_vault_path),
        &skill_id,
    ) {
        return SkillReadResult {
            success: false,
            skill: None,
            error: Some(err),
        };
    }
    match read_skill_detail(&app, &settings.chat_tools.skill_scan_paths, &skill_id) {
        Ok(skill) => SkillReadResult {
            success: true,
            skill: Some(skill),
            error: None,
        },
        Err(err) => SkillReadResult {
            success: false,
            skill: None,
            error: Some(err),
        },
    }
}

#[tauri::command]
#[allow(deprecated)]
pub fn chat_skills_open_folder(app: AppHandle) -> SkillOpenFolderResult {
    match user_skills_dir(&app) {
        Ok(dir) => {
            let path = dir.display().to_string();
            if let Err(err) = app.shell().open(&path, None) {
                SkillOpenFolderResult {
                    success: false,
                    path: Some(path),
                    error: Some(err.to_string()),
                }
            } else {
                SkillOpenFolderResult {
                    success: true,
                    path: Some(path),
                    error: None,
                }
            }
        }
        Err(err) => SkillOpenFolderResult {
            success: false,
            path: None,
            error: Some(err),
        },
    }
}

#[tauri::command]
pub fn chat_skills_import(app: AppHandle, path: String) -> SkillImportResult {
    let source = PathBuf::from(path);
    let skills_dir = match user_skills_dir(&app) {
        Ok(path) => path,
        Err(err) => {
            return SkillImportResult {
                success: false,
                skill: None,
                error: Some(err),
            }
        }
    };
    let result = if source.is_dir() {
        import_skill_dir(&source, &skills_dir)
    } else if source
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("zip"))
        .unwrap_or(false)
    {
        import_skill_zip(&source, &skills_dir)
    } else {
        Err("Skill import expects a folder or zip containing SKILL.md".to_string())
    };

    match result {
        Ok(meta) => SkillImportResult {
            success: true,
            skill: Some(meta),
            error: None,
        },
        Err(err) => SkillImportResult {
            success: false,
            skill: None,
            error: Some(err),
        },
    }
}

pub fn read_skill_detail(
    app: &AppHandle,
    extra_paths: &[String],
    skill_id: &str,
) -> Result<SkillDetail, String> {
    let registry = build_registry(app, extra_paths)?;
    let record = registry
        .find(skill_id)
        .ok_or_else(|| format!("Skill not found: {skill_id}"))?;
    Ok(SkillDetail {
        meta: record.meta.clone(),
        body: record.body.clone(),
    })
}

pub fn read_skill_record(
    app: &AppHandle,
    extra_paths: &[String],
    skill_id: &str,
) -> Result<SkillRecord, String> {
    let registry = build_registry(app, extra_paths)?;
    registry
        .find(skill_id)
        .cloned()
        .ok_or_else(|| format!("Skill not found: {skill_id}"))
}

fn import_skill_dir(source: &Path, skills_dir: &Path) -> Result<SkillMeta, String> {
    let skill_file = source.join("SKILL.md");
    if !skill_file.is_file() {
        return Err("Selected folder does not contain SKILL.md".to_string());
    }
    let raw =
        fs::read_to_string(&skill_file).map_err(|err| format!("Read SKILL.md failed: {err}"))?;
    let files = discover::index_skill_files(source)?;
    let mut warnings = Vec::new();
    let parsed = parse::parse_skill_record(&skill_file, &raw, "user", files, &mut warnings)?;
    let dest = skills_dir.join(&parsed.meta.id);
    copy_dir_recursive(source, &dest)?;
    Ok(SkillMeta {
        path: Some(dest.join("SKILL.md").display().to_string()),
        ..parsed.meta
    })
}

fn import_skill_zip(source: &Path, skills_dir: &Path) -> Result<SkillMeta, String> {
    let bytes = fs::read(source).map_err(|err| format!("Read zip failed: {err}"))?;
    let reader = Cursor::new(bytes);
    let mut archive =
        zip::ZipArchive::new(reader).map_err(|err| format!("Open zip failed: {err}"))?;
    let mut skill_raw = String::new();
    let mut skill_path = String::new();
    for i in 0..archive.len() {
        let mut file = archive.by_index(i).map_err(|err| err.to_string())?;
        if file.name().ends_with("SKILL.md") {
            file.read_to_string(&mut skill_raw)
                .map_err(|err| format!("Read SKILL.md in zip failed: {err}"))?;
            skill_path = file.name().to_string();
            break;
        }
    }
    if skill_raw.trim().is_empty() {
        return Err("Zip does not contain SKILL.md".to_string());
    }
    let parsed = parse_skill_markdown(&skill_raw, "user", None, Vec::new())?;
    let dest = skills_dir.join(&parsed.meta.id);
    fs::create_dir_all(&dest).map_err(|err| format!("create skill dir failed: {err}"))?;
    for i in 0..archive.len() {
        let mut file = archive.by_index(i).map_err(|err| err.to_string())?;
        if file.is_dir() {
            continue;
        }
        let name = file.name();
        let relative = if skill_path.contains('/') {
            let prefix = skill_path
                .rsplit_once('/')
                .map(|(prefix, _)| format!("{prefix}/"))
                .unwrap_or_default();
            name.strip_prefix(&prefix).unwrap_or(name)
        } else {
            name
        };
        if relative.contains("..") {
            continue;
        }
        let out = dest.join(relative);
        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent).map_err(|err| err.to_string())?;
        }
        let mut output = fs::File::create(&out).map_err(|err| err.to_string())?;
        std::io::copy(&mut file, &mut output).map_err(|err| err.to_string())?;
    }
    Ok(SkillMeta {
        path: Some(dest.join("SKILL.md").display().to_string()),
        ..parsed.meta
    })
}

fn copy_dir_recursive(from: &Path, to: &Path) -> Result<(), String> {
    fs::create_dir_all(to).map_err(|err| err.to_string())?;
    for entry in fs::read_dir(from).map_err(|err| err.to_string())? {
        let entry = entry.map_err(|err| err.to_string())?;
        let src = entry.path();
        let dst = to.join(entry.file_name());
        if src.is_dir() {
            copy_dir_recursive(&src, &dst)?;
        } else if src.is_file() {
            fs::copy(&src, &dst).map_err(|err| err.to_string())?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_skill_supports_recommended_tools_and_mcp_tools_alias() {
        let raw = r#"---
name: test-skill
description: Uses selected tools.
recommended-tools:
  - web_search
mcp-tools:
  - fetch
  - web_search
allowed-tools: Bash(git:*)
---
# Body
"#;

        let parsed = parse_skill_markdown(raw, "user", None, Vec::new()).unwrap();

        assert!(parsed.meta.recommended_tools.contains(&"fetch".to_string()));
        assert!(parsed
            .meta
            .recommended_tools
            .contains(&"web_search".to_string()));
        assert!(parsed
            .meta
            .recommended_tools
            .iter()
            .any(|tool| tool.contains("Bash")));
    }

    #[test]
    fn parse_skill_requires_name_and_description() {
        let err = parse_skill_markdown(
            r#"---
description: Missing name.
---
# Body
"#,
            "user",
            None,
            Vec::new(),
        )
        .expect_err("missing name should be rejected");

        assert!(err.contains("name"));
    }

    #[test]
    fn disable_model_invocation_parses_from_frontmatter() {
        let parsed = parse_skill_markdown(
            r#"---
name: manual-only
description: Only when invoked explicitly.
disable-model-invocation: true
---
"#,
            "user",
            None,
            Vec::new(),
        )
        .unwrap();
        assert!(parsed.meta.disable_model_invocation);
    }
}
