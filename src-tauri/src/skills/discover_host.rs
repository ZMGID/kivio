use std::{fs,path::{Path,PathBuf}};
use tauri::{AppHandle,Manager};
use super::SkillRegistry;
pub fn user_skills_dir(_app: &AppHandle) -> Result<PathBuf, String> {
    let dir = super::discover::kivio_skills_dir().ok_or_else(|| "home directory unavailable".to_string())?;
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

pub fn build_registry(app: &AppHandle, extra_paths: &[String]) -> Result<SkillRegistry, String> {
    build_registry_in(app, extra_paths, None)
}

pub fn build_registry_in(
    app: &AppHandle,
    extra_paths: &[String],
    project_cwd: Option<&Path>,
) -> Result<SkillRegistry, String> {
    build_registry_with_plugins(
        app,
        extra_paths,
        project_cwd,
        &crate::plugins::packages::active(),
    )
}


pub(crate) fn build_registry_with_plugins(
    app: &AppHandle, extra_paths: &[String], project_cwd: Option<&Path>,
    plugins: &[crate::plugins::packages::Resolved],
) -> Result<SkillRegistry,String> {
    super::discover::build_registry_with_plugins_at(bundled_skills_dir(app).as_deref(),extra_paths,project_cwd,plugins)
}
