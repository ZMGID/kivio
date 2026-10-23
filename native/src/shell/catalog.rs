//! Project/set persistence uses the same Rust catalog as the Tauri host.
use super::*;
use crate::group_catalog::{self, ChatProject, ChatSet};
use gpui_kit::component::notification::Notification;
use std::path::{Path, PathBuf};

impl Shell {
    pub(super) fn catalog_operation(
        &mut self,
        operation: impl FnOnce(&Path) -> Result<Option<String>, String> + Send + 'static,
        close_editor: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.catalog_busy {
            return;
        }
        self.catalog_busy = true;
        let directory = self.catalog_dir.clone();
        let editor = self.group_editor.as_ref().map(|editor| editor.name.clone());
        let work = cx.background_spawn(async move {
            let selected = operation(&directory)?;
            let projects = group_catalog::load_project_index(&directory)?.projects;
            let sets = group_catalog::load_set_index(&directory)?.sets;
            Ok::<_, String>((projects, sets, selected))
        });
        self.catalog_task = Some(cx.spawn_in(window, async move |owner, cx| {
            let result = work.await;
            let _ = owner.update_in(cx, |this, window, cx| {
                this.catalog_busy = false;
                match result {
                    Ok((projects, sets, selected)) => {
                        let first_load = !this.catalog_ready;
                        this.catalog_ready = true;
                        this.accept_catalog(projects, sets);
                        if first_load {
                            this.sync_group_memberships();
                        }
                        this.save_next(window, cx);
                        if let Some(selected) = selected {
                            this.selected_group = this
                                .groups
                                .iter()
                                .find(|group| group.key == selected)
                                .map(|group| group.id);
                        }
                        if close_editor
                            && this.group_editor.as_ref().map(|e| &e.name) == editor.as_ref()
                        {
                            this.group_editor = None;
                            window.close_dialog(cx);
                            window.focus(&this.focus, cx);
                        }
                    }
                    Err(error) => {
                        if let Some(current) = this.group_editor.as_mut()
                            && Some(&current.name) == editor.as_ref()
                        {
                            current.error = Some(error.into());
                        } else {
                            window.push_notification(
                                Notification::error(format!("无法保存或读取分组：{error}")),
                                cx,
                            );
                        }
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    pub(super) fn accept_catalog(&mut self, projects: Vec<ChatProject>, sets: Vec<ChatSet>) {
        let old = std::mem::take(&mut self.groups);
        self.groups = projects
            .into_iter()
            .map(|p| (p.id, p.name, true, p.root_path))
            .chain(sets.into_iter().map(|s| (s.id, s.name, false, None)))
            .map(|(key, name, project, directory)| {
                let id = old
                    .iter()
                    .find(|group| group.key == key)
                    .map(|group| group.id)
                    .unwrap_or_else(|| {
                        let id = self.next_group;
                        self.next_group += 1;
                        id
                    });
                Group {
                    id,
                    key,
                    name: name.into(),
                    project,
                    directory: directory.map(PathBuf::from),
                }
            })
            .collect();
        // Legacy folder names are persisted alongside project ids. Renaming a
        // project updates that compatibility field through the same record queue.
        for group in &self.groups {
            if group.project
                && old
                    .iter()
                    .any(|previous| previous.key == group.key && previous.name != group.name)
            {
                for chat in &mut self.conversations {
                    if chat.project == Some(group.id) {
                        chat.membership_dirty = true;
                        chat.record_dirty = true;
                        chat.dirty = true;
                    }
                }
            }
        }
    }

    pub(super) fn persist_group_order(
        &mut self,
        project: bool,
        ids: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.catalog_operation(
            move |directory| {
                if project {
                    group_catalog::reorder_projects(directory, &ids)?;
                } else {
                    let mut index = group_catalog::load_set_index(directory)?;
                    index.sets = group_catalog::reorder_by_ids(index.sets, &ids, |set| &set.id);
                    group_catalog::save_set_index(directory, &index)?;
                }
                Ok(None)
            },
            false,
            window,
            cx,
        );
    }
}
