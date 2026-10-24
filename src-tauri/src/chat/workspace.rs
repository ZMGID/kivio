//! Conversation workbench resolution shared by the native and legacy hosts.
use super::{
    conversation_data::Conversation,
    group_catalog::{self, ChatProject},
};
use std::path::{Path, PathBuf};

pub fn resolve_project(
    directory: &Path,
    conversation: &Conversation,
) -> Result<Option<ChatProject>, String> {
    if let Some(id) = conversation
        .project_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
    {
        group_catalog::validate_project_id(id)?;
        return group_catalog::load_project_index(directory)?
            .projects
            .into_iter()
            .find(|project| project.id == id)
            .map(Some)
            .ok_or_else(|| "项目不存在".into());
    }
    if let Some(name) = conversation
        .folder
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
    {
        return Ok(group_catalog::load_project_index(directory)?
            .projects
            .into_iter()
            .find(|project| project.name == name));
    }
    Ok(None)
}

pub fn resolve_working_directory(
    directory: &Path,
    conversation: &Conversation,
    ordinary_working_root: &str,
) -> Result<PathBuf, String> {
    if let Some(project) = resolve_project(directory, conversation)? {
        let root = project
            .root_path
            .as_deref()
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .ok_or_else(|| {
                format!(
                    "Project {} has no working directory configured",
                    project.name
                )
            })?;
        return Ok(PathBuf::from(root));
    }
    crate::working_directory::ordinary_workspace_directory(ordinary_working_root, &conversation.id)
}
