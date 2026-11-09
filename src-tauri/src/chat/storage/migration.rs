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
) -> Result<bool, String> {
    map_conversation_resource_paths(conversation, |raw| {
        for (source, target) in mappings {
            if let Ok(relative) = Path::new(raw).strip_prefix(source) {
                return Some(target.join(relative).to_string_lossy().into_owned());
            }
        }
        None
    })
}

pub(super) fn map_conversation_resource_paths(
    conversation: &mut Conversation,
    mut transform: impl FnMut(&str) -> Option<String>,
) -> Result<bool, String> {
    fn model_paths(
        message: &mut crate::chat::model::ModelMessage,
        transform: &mut impl FnMut(&mut String),
    ) {
        for part in &mut message.content {
            match part {
                crate::chat::model::MessagePart::Image {
                    path: Some(path), ..
                }
                | crate::chat::model::MessagePart::Video {
                    path: Some(path), ..
                }
                | crate::chat::model::MessagePart::ImageUrl { url: path, .. } => transform(path),
                crate::chat::model::MessagePart::ToolResult { artifacts, .. } => {
                    for artifact in artifacts {
                        if let Some(path) = &mut artifact.path {
                            transform(path);
                        }
                    }
                }
                _ => {}
            }
        }
    }
    fn wire_paths(value: &mut serde_json::Value, transform: &mut impl FnMut(&mut String)) {
        match value {
            serde_json::Value::Array(values) => {
                values.iter_mut().for_each(|v| wire_paths(v, transform))
            }
            serde_json::Value::Object(values) => {
                for (key, value) in values {
                    if matches!(key.as_str(), "path" | "file_path" | "url") {
                        if let Some(raw) = value.as_str() {
                            let mut path = raw.to_owned();
                            transform(&mut path);
                            *value = serde_json::Value::String(path);
                        }
                    } else {
                        wire_paths(value, transform);
                    }
                }
            }
            _ => {}
        }
    }
    let mut changed = false;
    let mut rewrite = |raw: &mut String| {
        if let Some(next) = transform(raw) {
            if *raw != next {
                *raw = next;
                changed = true;
            }
        }
    };
    if let Some(state) = &mut conversation.builtin_history {
        crate::chat::session_history::map_runtime_messages(state, |runtime| {
            if let Some(model) = runtime.model_message_mut() {
                model_paths(model, &mut rewrite);
            }
        })?;
    }
    let archived = conversation.builtin_history.iter_mut().flat_map(|state| {
        state
            .archived_branches
            .iter_mut()
            .flat_map(|branch| &mut branch.messages)
    });
    for message in conversation.messages.iter_mut().chain(archived) {
        for attachment in &mut message.attachments {
            rewrite(&mut attachment.path);
        }
        for artifact in message.artifacts.iter_mut().chain(
            message
                .tool_calls
                .iter_mut()
                .flat_map(|call| &mut call.artifacts),
        ) {
            if let Some(path) = &mut artifact.path {
                rewrite(path);
            }
        }
        for model in &mut message.model_messages {
            model_paths(model, &mut rewrite);
        }
        for wire in &mut message.api_messages {
            wire_paths(wire, &mut rewrite);
        }
    }
    let archived_contexts = conversation.builtin_history.iter_mut().flat_map(|state| {
        state
            .archived_branches
            .iter_mut()
            .map(|branch| &mut branch.context_state)
    });
    for context in std::iter::once(&mut conversation.context_state).chain(archived_contexts) {
        if let Some(replay) = context.summary.as_mut().and_then(|s| s.replay.as_mut()) {
            for message in &mut replay.messages {
                wire_paths(message, &mut rewrite);
            }
        }
    }
    Ok(changed)
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
                    && rewrite_conversation_artifact_paths(conversation, &mappings)?
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
    // 选了独立 worktree 的对话始终在 worktree 里读写，不回到项目根。
    if let Some(worktree) = conversation.worktree.as_ref() {
        return Ok(PathBuf::from(&worktree.path));
    }
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
    crate::native_tools::ordinary_workspace_directory(ordinary_working_root, &conversation.id)
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
