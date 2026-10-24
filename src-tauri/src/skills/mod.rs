#[cfg(test)]
use std::io::Cursor;
mod package_install;
pub use package_install::{download_skill_zip_into, download_skill_zip_into_checked, install_skill_zip_bytes};
#[cfg(test)]
use package_install::normalize_skill_download_url;
mod catalog;
mod discover;
mod discover_host;
pub mod parse;
mod runtime;
mod reader;
mod types;

pub use catalog::format_catalog;
pub(crate) use discover_host::build_registry_with_plugins;
pub use discover::{
    home_agents_skills_dir, kivio_skills_dir,
    legacy_app_data_skills_dir, read_skill_record,
};
pub use parse::parse_skill_markdown;
pub use runtime::{
    activate_skill, execute_skill_tool, extract_skill_name, lookup_skill, substitute_arguments, SkillRunContext,
};
pub use types::{
    slugify, SkillDetail, SkillImportResult, SkillListResult, SkillMeta, SkillOpenFolderResult,
    SkillReadResult, SkillRecord, SkillRegistry,
};

use std::{
    fs,
    path::{Path, PathBuf},
};

use tauri::{AppHandle, Manager};
use tauri_plugin_shell::ShellExt;

use crate::state::AppState;

#[tauri::command]
pub async fn chat_skills_list(
    app: AppHandle,
    skill_scan_paths: Option<Vec<String>>,
    project_cwd: Option<String>,
) -> SkillListResult {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let paths = skill_scan_paths
            .unwrap_or_else(|| state.settings_read().chat_tools.skill_scan_paths.clone());
        let cwd = project_cwd
            .as_deref()
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .map(PathBuf::from);
        match build_registry_in(&app, &paths, cwd.as_deref()) {
            Ok(registry) => {
                // 插件附属 skill 一并返回，技能页单独分区展示；
                // 开关仍由「扩展 → 插件」统一管理（前端禁止在技能页改插件 skill）。
                let skills = registry
                    .metas()
                    .into_iter()
                    .map(|mut meta| {
                        if crate::plugins::skill_owned_by_plugin(&meta.id).is_some() {
                            meta.source = "plugin".to_string();
                        }
                        meta
                    })
                    .collect();
                SkillListResult {
                    success: true,
                    skills,
                    error: None,
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
    })
    .await
    .unwrap_or_else(|err| SkillListResult {
        success: false,
        skills: Vec::new(),
        warnings: Vec::new(),
        error: Some(format!("Skill detection failed: {err}")),
    })
}

#[tauri::command]
pub async fn chat_skills_read(
    app: AppHandle,
    skill_id: String,
    project_cwd: Option<String>,
) -> SkillReadResult {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let settings = state.settings_read().chat_tools.clone();
        if let Some(err) =
            crate::settings::skill_global_unavailable_error(&settings, &skill_id, &skill_id)
        {
            return SkillReadResult {
                success: false,
                skill: None,
                error: Some(err),
            };
        }
        let cwd = project_cwd
            .as_deref()
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .map(PathBuf::from);
        match read_skill_detail_in(&app, &settings.skill_scan_paths, &skill_id, cwd.as_deref()) {
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
    })
    .await
    .unwrap_or_else(|error| SkillReadResult {
        success: false,
        skill: None,
        error: Some(format!("Skill read failed: {error}")),
    })
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
pub async fn chat_skills_import(app: AppHandle, path: String) -> SkillImportResult {
    tauri::async_runtime::spawn_blocking(move || import_skill(app, path))
        .await
        .unwrap_or_else(|error| SkillImportResult {
            success: false,
            skill: None,
            error: Some(format!("Skill import failed: {error}")),
        })
}

pub(crate) fn import_skill(app: AppHandle, path: String) -> SkillImportResult {
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

/// 卸载用户技能：先删 `~/.kivio/skills/<id>`，再回退旧 `{app_data}/skills/<id>`。
/// `~/.agents/skills` 是共享目录，不从这里删。内置与插件技能也无法经此删除。
#[tauri::command]
pub async fn chat_skills_uninstall(app: AppHandle, id: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        if id.is_empty() || id.contains('/') || id.contains('\\') || id.contains("..") {
            return Err("invalid skill id".to_string());
        }
        let mut candidates = Vec::new();
        if let Ok(dir) = user_skills_dir(&app) {
            candidates.push(dir.join(&id));
        }
        if let Some(dir) = legacy_app_data_skills_dir() {
            candidates.push(dir.join(&id));
        }
        let dir = candidates
            .into_iter()
            .find(|path| path.is_dir())
            .ok_or_else(|| {
                "技能不存在或不可删除（仅 ~/.kivio/skills 与旧个人目录可删除）".to_string()
            })?;
        fs::remove_dir_all(&dir).map_err(|err| format!("删除技能失败: {err}"))?;
        Ok(())
    })
    .await
    .map_err(|error| format!("chat_skills_uninstall failed: {error}"))?
}

/// 技能市场安装：从 ClawHub 下载链 / GitHub 仓库 / 直链 zip 下载 zip 并落盘。
/// 浏览/搜索/owner 消歧在前端完成，这里只负责下载 + 复用 `install_skill_zip_bytes`。
#[tauri::command]
pub async fn chat_skills_install_from_url(app: AppHandle, url: String) -> SkillImportResult {
    match install_skill_from_url(&app, &url).await {
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

async fn install_skill_from_url(app: &AppHandle, url: &str) -> Result<SkillMeta, String> {
    let skills_dir = user_skills_dir(app)?;
    download_skill_zip_into(url, &skills_dir).await
}

pub fn read_skill_detail(
    app: &AppHandle,
    extra_paths: &[String],
    skill_id: &str,
) -> Result<SkillDetail, String> {
    read_skill_detail_in(app, extra_paths, skill_id, None)
}

pub fn read_skill_detail_in(
    app: &AppHandle,
    extra_paths: &[String],
    skill_id: &str,
    project_cwd: Option<&Path>,
) -> Result<SkillDetail, String> {
    let registry = build_registry_in(app, extra_paths, project_cwd)?;
    let record = registry
        .find(skill_id)
        .ok_or_else(|| format!("Skill not found: {skill_id}"))?;
    let record = read_skill_record(record)?;
    Ok(SkillDetail {
        meta: record.meta,
        body: record.body,
    })
}

fn import_skill_dir(source: &Path, skills_dir: &Path) -> Result<SkillMeta, String> {
    let skill_file = source.join("SKILL.md");
    if !skill_file.is_file() {
        return Err("Selected folder does not contain SKILL.md".to_string());
    }
    let raw =
        fs::read_to_string(&skill_file).map_err(|err| format!("Read SKILL.md failed: {err}"))?;
    let files = Vec::new();
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
    install_skill_zip_bytes(bytes, skills_dir)
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
    fn url_normalize_github_repo_and_passthrough() {
        assert_eq!(
            normalize_skill_download_url("https://github.com/owner/repo"),
            "https://codeload.github.com/owner/repo/zip/HEAD"
        );
        assert_eq!(
            normalize_skill_download_url("https://github.com/owner/repo.git"),
            "https://codeload.github.com/owner/repo/zip/HEAD"
        );
        assert_eq!(
            normalize_skill_download_url("https://github.com/owner/repo/tree/dev"),
            "https://codeload.github.com/owner/repo/zip/dev"
        );
        // 直链 zip / clawhub 下载链原样返回
        let claw = "https://clawhub.ai/api/v1/download?slug=x&tag=latest&ownerHandle=y";
        assert_eq!(normalize_skill_download_url(claw), claw);
        assert_eq!(
            normalize_skill_download_url("https://example.com/a.zip"),
            "https://example.com/a.zip"
        );
    }

    fn zip_with_skill(id: &str) -> Vec<u8> {
        let mut buf = Cursor::new(Vec::new());
        {
            let mut z = zip::ZipWriter::new(&mut buf);
            let opts = zip::write::SimpleFileOptions::default();
            z.start_file("SKILL.md", opts).unwrap();
            let md = format!("---\nname: {id}\ndescription: A test skill.\n---\n# Body\n");
            std::io::Write::write_all(&mut z, md.as_bytes()).unwrap();
            z.start_file("script.py", opts).unwrap();
            std::io::Write::write_all(&mut z, b"print('hi')\n").unwrap();
            z.finish().unwrap();
        }
        buf.into_inner()
    }

    #[test]
    fn install_skill_zip_bytes_lands_skill_and_files() {
        let dir = std::env::temp_dir().join(format!("kivio-skilltest-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let meta = install_skill_zip_bytes(zip_with_skill("zip-skill"), &dir).unwrap();
        assert_eq!(meta.id, "zip-skill");
        assert!(dir.join("zip-skill/SKILL.md").is_file());
        assert!(dir.join("zip-skill/script.py").is_file());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn install_skill_zip_bytes_bad_zip_errors_without_dir() {
        let dir = std::env::temp_dir().join(format!("kivio-skilltest-bad-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let err = install_skill_zip_bytes(b"not a zip".to_vec(), &dir);
        assert!(err.is_err());
        // 目录里不该留下任何技能子目录
        let leftovers: Vec<_> = fs::read_dir(&dir).unwrap().flatten().collect();
        assert!(leftovers.is_empty(), "bad zip left files behind");

        let _ = fs::remove_dir_all(&dir);
    }

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

pub use discover_host::{build_registry, build_registry_in, user_skills_dir};
