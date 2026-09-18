use super::*;

pub(super) fn has_non_empty_value(value: Option<&str>) -> bool {
    value.is_some_and(|value| !value.trim().is_empty())
}

fn legacy_folder_is_project(app: &AppHandle, folder: Option<&str>) -> Result<bool, String> {
    let Some(folder) = folder.map(str::trim).filter(|folder| !folder.is_empty()) else {
        return Ok(false);
    };
    Ok(find_project_by_name(app, folder)?.is_some())
}

pub(super) fn conversation_has_project_binding(
    app: &AppHandle,
    conversation: &Conversation,
) -> Result<bool, String> {
    if has_non_empty_value(conversation.project_id.as_deref()) {
        return Ok(true);
    }
    legacy_folder_is_project(app, conversation.folder.as_deref())
}

pub(super) fn conversation_list_item_has_project_binding(
    app: &AppHandle,
    item: &ConversationListItem,
) -> Result<bool, String> {
    if has_non_empty_value(item.project_id.as_deref()) {
        return Ok(true);
    }
    legacy_folder_is_project(app, item.folder.as_deref())
}

pub(crate) fn rewrite_conversation_artifact_paths(
    conversation: &mut Conversation,
    mappings: &[(PathBuf, PathBuf)],
) -> bool {
    fn rewrite(path: &mut Option<String>, mappings: &[(PathBuf, PathBuf)]) -> bool {
        let Some(raw) = path.as_deref() else {
            return false;
        };
        let current = Path::new(raw);
        for (source, target) in mappings {
            if let Ok(relative) = current.strip_prefix(source) {
                *path = Some(target.join(relative).to_string_lossy().to_string());
                return true;
            }
        }
        false
    }

    let mut changed = false;
    for message in &mut conversation.messages {
        for artifact in &mut message.artifacts {
            changed |= rewrite(&mut artifact.path, mappings);
        }
        for tool_call in &mut message.tool_calls {
            for artifact in &mut tool_call.artifacts {
                changed |= rewrite(&mut artifact.path, mappings);
            }
        }
    }
    changed
}

pub async fn migrate_ordinary_conversation_workspaces(
    app: &AppHandle,
    old_root: &str,
    new_root: &str,
) -> Result<(), String> {
    if old_root.trim() == new_root.trim() {
        return Ok(());
    }

    struct WorkspaceMigration {
        conversation_index: usize,
        old_dir: PathBuf,
        legacy_dir: PathBuf,
        new_dir: PathBuf,
    }

    crate::chat::repository::repository(app)
        .bulk_mutate_loaded(app, |conversations| {
            let mut migrations = Vec::new();
            for (conversation_index, conversation) in conversations.iter().enumerate() {
                if conversation_has_project_binding(app, conversation)? {
                    continue;
                }
                migrations.push(WorkspaceMigration {
                    conversation_index,
                    old_dir: crate::native_tools::conversation_workspace_directory(
                        old_root,
                        &conversation.id,
                    )?,
                    legacy_dir: crate::native_tools::legacy_outputs_dir(&conversation.id)?,
                    new_dir: crate::native_tools::conversation_workspace_directory(
                        new_root,
                        &conversation.id,
                    )?,
                });
            }

            // Validate every conversation before moving the first file. This prevents a
            // later name conflict from leaving earlier conversations on the new root.
            for migration in &migrations {
                if migration.old_dir.exists() {
                    crate::native_tools::preflight_directory_merge(
                        &migration.old_dir,
                        &migration.new_dir,
                    )?;
                }
                if migration.legacy_dir.exists() {
                    crate::native_tools::preflight_directory_merge(
                        &migration.legacy_dir,
                        &migration.new_dir,
                    )?;
                    if migration.old_dir.exists() {
                        crate::native_tools::preflight_directory_merge(
                            &migration.legacy_dir,
                            &migration.old_dir,
                        )?;
                    }
                }
            }

            let mut changed = Vec::new();
            for migration in migrations {
                let mut mappings = Vec::new();
                if migration.old_dir.exists() {
                    crate::native_tools::merge_directory_without_overwrite(
                        &migration.old_dir,
                        &migration.new_dir,
                    )?;
                    mappings.push((migration.old_dir, migration.new_dir.clone()));
                }
                if migration.legacy_dir.exists() {
                    crate::native_tools::merge_directory_without_overwrite(
                        &migration.legacy_dir,
                        &migration.new_dir,
                    )?;
                    mappings.push((migration.legacy_dir, migration.new_dir.clone()));
                }
                let conversation = &mut conversations[migration.conversation_index];
                if !mappings.is_empty()
                    && rewrite_conversation_artifact_paths(conversation, &mappings)
                {
                    changed.push(conversation.id.clone());
                }
            }
            Ok(changed)
        })
        .await
        .map(|_| ())
        .map_err(crate::chat::repository::repository_error)
}

pub fn resolve_conversation_working_directory(
    app: &AppHandle,
    conversation: &Conversation,
    ordinary_working_root: &str,
) -> Result<PathBuf, String> {
    if let Some(project) = resolve_conversation_project(app, conversation)? {
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
    crate::native_tools::conversation_workspace_directory(ordinary_working_root, &conversation.id)
}

pub fn resolve_conversation_project(
    app: &AppHandle,
    conversation: &Conversation,
) -> Result<Option<ChatProject>, String> {
    if let Some(project_id) = conversation
        .project_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
    {
        return find_project_by_id(app, project_id).map(Some);
    }
    if let Some(folder) = conversation
        .folder
        .as_deref()
        .map(str::trim)
        .filter(|folder| !folder.is_empty())
    {
        return find_project_by_name(app, folder);
    }
    Ok(None)
}
