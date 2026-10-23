//! The window keeps full canonical records; a single queue owns disk writes.
use super::*;
use crate::conversation_data::{AgentRuntimeKind, Conversation as Record, ConversationPin};
use crate::conversation_files::ConversationStore;
use gpui_kit::component::notification::Notification;
use std::{
    collections::{HashMap, VecDeque},
    path::PathBuf,
};

pub(super) struct Persistence {
    pub(super) store: ConversationStore,
    drafts_path: PathBuf,
    pub ready: bool,
    pub(super) busy: bool,
    pub(super) task: Option<Task<()>>,
    drafts: HashMap<String, String>,
    pub error: Option<String>,
    pub(super) quitting: bool,
    deleting: Option<usize>,
    draft_retry: bool,
    pub(super) pins: crate::group_catalog::ConversationPins,
    pin_writes: VecDeque<(String, Vec<ConversationPin>)>,
    group_delete: Option<(usize, String, bool)>,
}
impl Persistence {
    pub fn new(directory: PathBuf) -> Self {
        Self {
            store: ConversationStore::new(directory.clone()),
            drafts_path: directory.join("drafts.json"),
            ready: false,
            busy: false,
            task: None,
            drafts: HashMap::new(),
            error: None,
            quitting: false,
            deleting: None,
            draft_retry: false,
            pins: HashMap::new(),
            pin_writes: VecDeque::new(),
            group_delete: None,
        }
    }
}

impl Shell {
    pub(super) fn load_conversations(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.persistence.busy {
            return;
        }
        self.persistence.busy = true;
        let store = self.persistence.store.clone();
        let path = self.persistence.drafts_path.clone();
        let directory = self.catalog_dir.clone();
        let work = cx.background_spawn(async move {
            let records = store.load_all().map_err(|e| e.to_string())?;
            let drafts: HashMap<String, String> = match std::fs::read(&path) {
                Ok(bytes) => {
                    serde_json::from_slice(&bytes).map_err(|e| format!("读取草稿失败：{e}"))?
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => HashMap::new(),
                Err(e) => return Err(format!("读取草稿失败：{e}")),
            };
            let pins = crate::group_catalog::load_conversation_pins(&directory)?;
            Ok::<_, String>((records, drafts, pins))
        });
        self.persistence.task = Some(cx.spawn_in(window, async move |owner, cx| {
            let result = work.await;
            let _ = owner.update_in(cx, |this, window, cx| {
                this.persistence.busy = false;
                match result {
                    Ok((mut records, drafts, pins)) => {
                        this.persistence.drafts = drafts;
                        this.persistence.pins = pins;
                        records.sort_by_key(|record| record.updated_at);
                        // Keep the initial pane until replacements exist, so observers
                        // always have a valid current entity.
                        let initial = this.active;
                        let had_records = !records.is_empty();
                        for record in records {
                            this.new_chat(window, cx);
                            let chat = &mut this.conversations[0];
                            chat.title = record.title.clone().into();
                            chat.title_is_user_set =
                                record.title != "新对话" || !record.messages.is_empty();
                            chat.runtime = if record.agent_runtime.kind == AgentRuntimeKind::Chat {
                                Runtime::Chat
                            } else {
                                Runtime::Agent
                            };
                            chat.locked = !record.messages.is_empty()
                                || record.agent_runtime.kind == AgentRuntimeKind::External;
                            chat.archived = record.archived;
                            let draft = this
                                .persistence
                                .drafts
                                .get(&record.id)
                                .cloned()
                                .unwrap_or_default();
                            chat.view.update(cx, |view, cx| {
                                view.configure(
                                    this.providers.clone(),
                                    Some((record.provider_id.clone(), record.model.clone())),
                                    record.id.clone(),
                                    this.catalog_dir.clone(),
                                );
                                view.set_history_only(
                                    record.provider_id != "native-preview"
                                        && record.agent_runtime.kind != AgentRuntimeKind::Chat,
                                );
                                if record.provider_id == "native-preview" {
                                    view.set_sample_mode(true, cx);
                                }
                                view.set_fork_origin(record.forked_from.as_ref());
                                view.restore(record.messages.clone(), draft, window, cx);
                            });
                            chat.record = record;
                            chat.persisted = true;
                        }
                        if had_records {
                            this.conversations.retain(|chat| chat.id != initial);
                        }
                        this.sync_group_memberships();
                        this.persistence.ready = true;
                        this.persistence.error = None;
                    }
                    Err(error) => this.persistence_failure(error, window, cx),
                }
                cx.notify();
            });
        }));
    }

    pub(super) fn sync_group_memberships(&mut self) {
        for chat in &mut self.conversations {
            chat.project = self
                .groups
                .iter()
                .find(|group| {
                    Some(&group.key) == chat.record.project_id.as_ref()
                        || (chat.record.project_id.is_none()
                            && group.project
                            && chat.record.folder.as_deref() == Some(group.name.as_ref()))
                })
                .map(|group| group.id);
            chat.set = self
                .groups
                .iter()
                .find(|group| Some(&group.key) == chat.record.set_id.as_ref())
                .map(|group| group.id);
        }
    }

    pub(super) fn mark_dirty(&mut self, id: usize, window: &mut Window, cx: &mut Context<Self>) {
        if !self.persistence.ready {
            return;
        }
        if let Some(chat) = self.conversations.iter_mut().find(|chat| chat.id == id) {
            chat.dirty = true;
            chat.record_dirty = true;
        }
        if self.persistence.error.is_none() {
            self.save_next(window, cx);
        }
    }

    pub(super) fn mark_draft_dirty(
        &mut self,
        id: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.persistence.ready {
            return;
        }
        if let Some(chat) = self.conversations.iter_mut().find(|chat| chat.id == id) {
            chat.dirty = true;
        }
        if self.persistence.error.is_none() {
            self.save_next(window, cx);
        }
    }

    pub(super) fn capture_record(&self, id: usize, cx: &App) -> (Record, String) {
        let chat = self
            .conversations
            .iter()
            .find(|chat| chat.id == id)
            .unwrap();
        let mut record = chat.record.clone();
        record.title = chat.title.to_string();
        record.archived = chat.archived;
        // Metadata-only operations retain every provider/tool/CLI field.
        let (messages, draft) = chat.view.read(cx).snapshot(cx);
        if messages.len() < record.messages.len() {
            record.invalidate_summary_from(messages.len());
        }
        record.messages = messages;
        if self.catalog_ready && chat.membership_dirty {
            record.project_id = chat.project.and_then(|id| {
                self.groups
                    .iter()
                    .find(|g| g.id == id)
                    .map(|g| g.key.clone())
            });
            record.folder = chat.project.and_then(|id| {
                self.groups
                    .iter()
                    .find(|g| g.id == id)
                    .map(|g| g.name.to_string())
            });
            record.set_id = chat.set.and_then(|id| {
                self.groups
                    .iter()
                    .find(|g| g.id == id)
                    .map(|g| g.key.clone())
            });
        }
        if record.agent_runtime.kind != AgentRuntimeKind::External {
            record.agent_runtime.kind = match chat.runtime {
                Runtime::Agent => AgentRuntimeKind::Builtin,
                Runtime::Chat => AgentRuntimeKind::Chat,
            };
        }
        (record, draft)
    }

    pub(super) fn save_next(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.persistence.busy || !self.persistence.ready {
            return;
        }
        let Some(id) = self.persistence.deleting.or_else(|| {
            self.conversations
                .iter()
                .find(|chat| chat.dirty)
                .map(|chat| chat.id)
        }) else {
            if !self.persistence.pin_writes.is_empty() || self.persistence.group_delete.is_some() {
                self.save_catalog_changes(window, cx);
            } else if self.persistence.draft_retry {
                self.retry_drafts(window, cx);
            } else if self.persistence.quitting {
                self.terminal = None;
                self.terminal_picker = None;
                cx.quit();
            }
            return;
        };
        let deleting = self.persistence.deleting == Some(id);
        let (record, draft) = self.capture_record(id, cx);
        let chat = self
            .conversations
            .iter_mut()
            .find(|chat| chat.id == id)
            .unwrap();
        chat.dirty = false;
        let persisted = chat.persisted;
        let record_dirty = chat.record_dirty;
        let membership_dirty = chat.membership_dirty;
        chat.membership_dirty = false;
        chat.record_dirty = false;
        if deleting || draft.is_empty() {
            self.persistence.drafts.remove(&record.id);
        } else {
            self.persistence.drafts.insert(record.id.clone(), draft);
        }
        let drafts = self.persistence.drafts.clone();
        let path = self.persistence.drafts_path.clone();
        let store = self.persistence.store.clone();
        self.persistence.busy = true;
        let work = cx.background_spawn(async move {
            let content = serde_json::to_string_pretty(&drafts).map_err(|e| e.to_string())?;
            let mut warnings = Vec::new();
            let saved = if deleting {
                if persisted {
                    warnings = store
                        .delete(&record.id, record.revision)
                        .map_err(|e| e.to_string())?;
                }
                None
            } else {
                let revision = record.revision;
                Some(
                    if persisted && !record_dirty {
                        Ok(record)
                    } else if persisted {
                        store.save(record, revision)
                    } else {
                        store.create(record)
                    }
                    .map_err(|e| e.to_string())?,
                )
            };
            // Commit history before clearing a sent draft. A draft failure must
            // still acknowledge the new record revision, so retry cannot conflict.
            let draft_error = crate::file_io::atomic_write(&path, &content, "drafts").err();
            Ok::<_, String>((saved, draft_error, warnings))
        });
        self.persistence.task = Some(cx.spawn_in(window, async move |owner, cx| {
            let result = work.await;
            let _ = owner.update_in(cx, |this, window, cx| {
                this.persistence.busy = false;
                match result {
                    Ok((Some(saved), draft_error, _)) => {
                        if let Some(chat) = this.conversations.iter_mut().find(|chat| chat.id == id)
                        {
                            // A new draft/edit may already be queued; only advance its
                            // confirmed baseline, never replace the live view.
                            chat.record = saved;
                            chat.persisted = true;
                        }
                        if let Some(error) = draft_error {
                            if let Some(chat) =
                                this.conversations.iter_mut().find(|chat| chat.id == id)
                            {
                                chat.dirty = true;
                            }
                            this.persistence_failure(error, window, cx);
                        } else {
                            this.save_next(window, cx);
                        }
                    }
                    Ok((None, draft_error, warnings)) => {
                        this.persistence.deleting = None;
                        if let Some(chat) = this.conversations.iter().find(|chat| chat.id == id) {
                            let key = &chat.record.id;
                            for pins in this.persistence.pins.values_mut() {
                                pins.retain(|pin| &pin.id != key);
                            }
                            for (_, pins) in &mut this.persistence.pin_writes {
                                pins.retain(|pin| &pin.id != key);
                            }
                        }
                        this.conversations.retain(|chat| chat.id != id);
                        for warning in warnings {
                            window.push_notification(
                                Notification::warning(format!("对话已删除，{warning}")),
                                cx,
                            );
                        }
                        if this.active == id {
                            if let Some(chat) = this.conversations.first() {
                                this.select(chat.id, window, cx);
                            } else {
                                this.new_chat(window, cx);
                            }
                        }
                        if let Some(error) = draft_error {
                            this.persistence.draft_retry = true;
                            this.persistence_failure(error, window, cx);
                        } else {
                            this.save_next(window, cx);
                        }
                    }
                    Err(error) => {
                        if let Some(chat) = this.conversations.iter_mut().find(|chat| chat.id == id)
                        {
                            chat.dirty = true;
                            chat.record_dirty |= record_dirty;
                            chat.membership_dirty |= membership_dirty;
                        }
                        // Retain a failed deletion as the next operation; retry must
                        // not turn it into an ordinary save and resurrect the record.
                        this.persistence_failure(error, window, cx);
                    }
                }
                cx.notify();
            });
        }));
    }

    fn retry_drafts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let path = self.persistence.drafts_path.clone();
        let drafts = self.persistence.drafts.clone();
        self.persistence.busy = true;
        let work = cx.background_spawn(async move {
            let content = serde_json::to_string_pretty(&drafts).map_err(|e| e.to_string())?;
            crate::file_io::atomic_write(&path, &content, "drafts")
        });
        self.persistence.task = Some(cx.spawn_in(window, async move |owner, cx| {
            let result = work.await;
            let _ = owner.update_in(cx, |this, window, cx| {
                this.persistence.busy = false;
                match result {
                    Ok(()) => {
                        this.persistence.draft_retry = false;
                        this.save_next(window, cx);
                    }
                    Err(error) => this.persistence_failure(error, window, cx),
                }
                cx.notify();
            });
        }));
    }

    pub(super) fn queue_pins(
        &mut self,
        group: String,
        pins: Vec<ConversationPin>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.persistence.ready {
            return;
        }
        self.persistence.pins.insert(group.clone(), pins.clone());
        self.persistence.pin_writes.push_back((group, pins));
        if self.persistence.error.is_none() {
            self.save_next(window, cx);
        }
        cx.notify();
    }

    pub(super) fn queue_group_deletion(
        &mut self,
        id: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.catalog_busy || !self.persistence.ready || self.persistence.error.is_some() {
            return;
        }
        let Some(group) = self.groups.iter().find(|group| group.id == id) else {
            return;
        };
        self.persistence.group_delete = Some((id, group.key.clone(), group.project));
        self.catalog_busy = true;
        if self.selected_group == Some(id) {
            self.selected_group = None;
        }
        // Persist every affected full record first. If any save fails, the group
        // stays in its index and the same queue can safely retry the remaining work.
        for chat in &mut self.conversations {
            if chat.project == Some(id) || chat.set == Some(id) {
                chat.project = None;
                chat.set = None;
                chat.membership_dirty = true;
                chat.record_dirty = true;
                chat.dirty = true;
            }
        }
        self.save_next(window, cx);
        cx.notify();
    }

    fn save_catalog_changes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let pin_write = self.persistence.pin_writes.front().cloned();
        let deletion = self.persistence.group_delete.clone();
        let directory = self.catalog_dir.clone();
        self.persistence.busy = true;
        let work = cx.background_spawn(async move {
            use crate::group_catalog as catalog;
            if let Some((group, pins)) = pin_write {
                catalog::set_conversation_pins(&directory, &group, pins)?;
                return Ok::<_, String>(None);
            }
            let (_, key, project) = deletion.expect("pending catalog operation");
            let mut projects = catalog::load_project_index(&directory)?;
            let mut sets = catalog::load_set_index(&directory)?;
            if project {
                projects.projects.retain(|group| group.id != key);
                catalog::save_project_index(&directory, &projects)?;
            } else {
                sets.sets.retain(|group| group.id != key);
                catalog::save_set_index(&directory, &sets)?;
            }
            // The group index has committed; optional pin cleanup cannot turn
            // success into a misleading failure of the already-removed group.
            let warning = catalog::set_conversation_pins(&directory, &key, Vec::new()).err();
            Ok(Some((projects.projects, sets.sets, warning)))
        });
        self.persistence.task = Some(cx.spawn_in(window, async move |owner, cx| {
            let result = work.await;
            let _ = owner.update_in(cx, |this, window, cx| {
                this.persistence.busy = false;
                match result {
                    Ok(None) => {
                        this.persistence.pin_writes.pop_front();
                        this.save_next(window, cx);
                    }
                    Ok(Some((projects, sets, warning))) => {
                        if let Some((id, key, _)) = this.persistence.group_delete.take() {
                            this.persistence.pins.remove(&key);
                            this.collapsed_groups.remove(&id);
                        }
                        this.accept_catalog(projects, sets);
                        this.catalog_busy = false;
                        if let Some(warning) = warning {
                            window.push_notification(
                                Notification::warning(format!(
                                    "分组已删除，固定位置未能清理：{warning}"
                                )),
                                cx,
                            );
                        }
                        this.save_next(window, cx);
                    }
                    Err(error) => this.persistence_failure(error, window, cx),
                }
                cx.notify();
            });
        }));
    }

    fn persistence_failure(&mut self, error: String, window: &mut Window, cx: &mut Context<Self>) {
        self.persistence.quitting = false;
        self.persistence.error = Some(error.clone());
        window.push_notification(Notification::error(format!("对话未保存：{error}")), cx);
    }

    pub(super) fn retry_persistence(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.persistence.error = None;
        if self.persistence.ready {
            self.save_next(window, cx);
        } else {
            self.load_conversations(window, cx);
        }
    }

    pub(crate) fn request_quit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.persistence.ready {
            self.terminal = None;
            self.terminal_picker = None;
            cx.quit();
            return;
        }
        if self.persistence.error.is_some() {
            window.push_notification(Notification::error("存在未保存的更改，请先重试保存。"), cx);
            return;
        }
        self.persistence.quitting = true;
        for chat in &mut self.conversations {
            if chat.view.read(cx).is_streaming(cx) || chat.view.read(cx).is_busy(cx) {
                chat.view.update(cx, |view, cx| view.stop(cx));
                chat.dirty = true;
                chat.record_dirty = true;
            }
        }
        self.save_next(window, cx);
    }

    pub(super) fn delete_current(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.delete_conversation(self.active, window, cx);
    }

    pub(super) fn delete_conversation(
        &mut self,
        id: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.persistence.deleting.is_some() || self.persistence.error.is_some() {
            return;
        }
        let Some(chat) = self.conversations.iter().find(|chat| chat.id == id) else {
            return;
        };
        // External CLI owns additional bindings/workspaces; retain its protected lifecycle.
        if chat.record.agent_runtime.kind == AgentRuntimeKind::External {
            window.push_notification(
                Notification::error("CLI 对话的运行时清理尚未接入，请在原客户端删除。"),
                cx,
            );
            return;
        }
        let title = format!("删除“{}”？", chat.title);
        let owner = cx.entity().downgrade();
        window.open_alert_dialog(cx, move |dialog, _, _| {
            let owner = owner.clone();
            dialog
                .title(title.clone())
                .confirm()
                .description("此操作会永久删除对话记录、草稿和此对话的附件。正在进行的回复会停止。")
                .button_props(
                    gpui_kit::component::dialog::DialogButtonProps::default()
                        .ok_text("删除对话")
                        .cancel_text("取消")
                        .ok_variant(gpui_kit::component::button::ButtonVariant::Danger),
                )
                .on_ok(move |_, window, cx| {
                    let _ = owner.update(cx, |this, cx| {
                        if this.persistence.deleting.is_some() || this.persistence.error.is_some() {
                            return;
                        }
                        if let Some(chat) = this.conversations.iter().find(|chat| chat.id == id) {
                            chat.view.update(cx, |view, cx| {
                                view.stop(cx);
                                view.set_history_only(true);
                            });
                            this.persistence.deleting = Some(id);
                            this.save_next(window, cx);
                        }
                    });
                    true
                })
        });
    }
}

#[cfg(test)]
mod tests {
    use super::Shell;
    use gpui_kit::{AppContext, TestAppContext, WindowOptions};

    // Full canonical chat records (including provider payloads) can be deleted.
    // A failed authoritative removal must retry deletion rather than save/recreate it.
    #[gpui_kit::test]
    fn confirmed_delete_retries_and_cleans_owned_files_without_touching_siblings(
        cx: &mut TestAppContext,
    ) {
        let data = tempfile::tempdir().unwrap();
        let store = crate::conversation_files::ConversationStore::new(data.path().to_owned());
        let record: crate::conversation_data::Conversation = serde_json::from_value(serde_json::json!({
            "id":"conv_delete", "title":"将删除的记录", "provider_id":"native-preview", "model":"sample",
            "created_at":1, "updated_at":1, "agent_runtime":{"kind":"chat"},
            "messages":[{"id":"message", "role":"assistant", "content":"保留的业务记录", "timestamp":1,
                "model_messages":[{"role":"assistant", "content":[{"type":"text", "text":"provider payload"}]}]}]
        })).unwrap();
        let record = store.create(record).unwrap();
        let sibling = store
            .create(crate::conversation_data::Conversation::new(
                "native-preview".into(),
                "sample".into(),
            ))
            .unwrap();
        let attachment_dir = data.path().join("conv_delete_attachments");
        std::fs::create_dir(&attachment_dir).unwrap();
        std::fs::write(attachment_dir.join("attachment.txt"), "owned attachment").unwrap();
        std::fs::write(
            data.path().join("conv_delete.draft.jsonl"),
            "owned draft log",
        )
        .unwrap();
        crate::group_catalog::set_conversation_pins(
            data.path(),
            "group",
            vec![
                crate::conversation_data::ConversationPin {
                    id: record.id.clone(),
                    row: 0,
                },
                crate::conversation_data::ConversationPin {
                    id: sibling.id.clone(),
                    row: 1,
                },
            ],
        )
        .unwrap();
        let (window, shell) = cx.update(|cx| {
            gpui_kit::init(cx);
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| Shell::new(data.path().to_owned(), window, cx))
            })
            .unwrap()
        });
        cx.run_until_parked();
        let id = shell.read_with(cx, |shell, _| {
            shell
                .conversations
                .iter()
                .find(|chat| chat.record.id == record.id)
                .unwrap()
                .id
        });
        window
            .update(cx, |_, window, cx| {
                shell.update(cx, |shell, cx| shell.delete_conversation(id, window, cx))
            })
            .unwrap();
        cx.simulate_keystrokes(window, "escape");
        cx.run_until_parked();
        assert!(
            store.load(&record.id).is_ok(),
            "cancel must keep the conversation"
        );
        let file = data.path().join("conv_delete.json");
        let bytes = std::fs::read(&file).unwrap();
        std::fs::remove_file(&file).unwrap();
        std::fs::create_dir(&file).unwrap();
        window
            .update(cx, |_, window, cx| {
                shell.update(cx, |shell, cx| shell.delete_conversation(id, window, cx))
            })
            .unwrap();
        cx.simulate_keystrokes(window, "enter");
        cx.run_until_parked();
        shell.read_with(cx, |shell, _| {
            assert!(shell.persistence.error.is_some());
            assert!(shell.conversations.iter().any(|chat| chat.id == id));
        });
        std::fs::remove_dir(&file).unwrap();
        std::fs::write(&file, bytes).unwrap();
        window
            .update(cx, |_, window, cx| {
                shell.update(cx, |shell, cx| shell.retry_persistence(window, cx))
            })
            .unwrap();
        cx.run_until_parked();
        assert!(!file.exists());
        assert!(!attachment_dir.exists());
        assert!(!data.path().join("conv_delete.draft.jsonl").exists());
        assert!(store.load(&sibling.id).is_ok());
        let pins = crate::group_catalog::load_conversation_pins(data.path()).unwrap();
        assert_eq!(pins["group"].len(), 1);
        assert_eq!(pins["group"][0].id, sibling.id);
        shell.read_with(cx, |shell, _| {
            assert!(shell.persistence.error.is_none());
            assert!(!shell.conversations.iter().any(|chat| chat.id == id));
        });
    }
}
