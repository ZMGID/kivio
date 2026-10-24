//! Kivio's conversation library, using its shared query and durable store.
use gpui_kit::component::{
    ActiveTheme, Disableable, Selectable, Sizable, StyledExt, WindowExt,
    button::{Button, ButtonVariants},
    checkbox::Checkbox,
    input::{Input, InputEvent, InputState},
    menu::{DropdownMenu, PopupMenuItem},
    switch::Switch,
};
use gpui_kit::{prelude::*, *};
use kivio_model_runtime::{
    conversation_data::{ConversationListItem, ConversationSearchHit},
    conversation_files::ConversationStore,
    conversation_library::{self, ConversationLibraryQuery},
    conversation_search,
};

pub struct OpenConversation(pub String);
pub struct LibraryPage {
    english_placeholders: bool,
    directory: std::path::PathBuf,
    state: Entity<crate::state::AppState>,
    operation: Option<Task<()>>,
    editor: Option<WeakEntity<ConversationEditor>>,
    search: Entity<InputState>,
    shelf: &'static str,
    sort: &'static str,
    ascending: bool,
    runtime_kind: &'static str,
    group: Option<(bool, String)>,
    groups: Vec<(bool, String, String)>,
    mutating: bool,
    full_text: bool,
    page: usize,
    total: usize,
    rows: Vec<ConversationSearchHit>,
    selected: std::collections::BTreeSet<String>,
    busy: bool,
    error: Option<String>,
    export_status: Option<String>,
    task: Option<Task<()>>,
    _search: Subscription,
}
impl EventEmitter<OpenConversation> for LibraryPage {}
impl LibraryPage {
    pub fn new(
        directory: std::path::PathBuf,
        state: Entity<crate::state::AppState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder(crate::i18n::text("搜索标题、预览和正文")));
        let subscription = cx.subscribe(&search, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.page = 0;
                this.refresh(cx);
            }
        });
        let mut this = Self {
            english_placeholders: crate::i18n::is_english(),
            directory,
            state,
            operation: None,
            editor: None,
            search,
            shelf: "all",
            sort: "updated",
            ascending: false,
            runtime_kind: "",
            group: None,
            groups: vec![],
            mutating: false,
            full_text: true,
            page: 0,
            total: 0,
            rows: vec![],
            selected: Default::default(),
            busy: false,
            error: None,
            export_status: None,
            task: None,
            _search: subscription,
        };
        this.refresh(cx);
        this
    }
    pub(crate) fn is_busy(&self) -> bool {
        self.mutating
    }

    pub(crate) fn has_unsaved_changes(&self, cx: &App) -> bool {
        self.editor
            .as_ref()
            .and_then(WeakEntity::upgrade)
            .is_some_and(|editor| {
                let editor = editor.read(cx);
                !editor.delete && editor.title.read(cx).value().to_string() != editor.original
            })
    }

    fn edit(
        &mut self,
        id: String,
        title: String,
        delete: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.mutating || self.busy {
            return;
        }
        let owner = cx.weak_entity();
        let editor = cx.new(|cx| ConversationEditor {
            owner,
            id,
            title: cx.new(|cx| InputState::new(window, cx).default_value(title.clone())),
            original: title,
            delete,
            busy: false,
            error: None,
            task: None,
        });
        self.editor = Some(editor.downgrade());
        let title_focus = editor.read(cx).title.read(cx).focus_handle(cx);
        window.open_dialog(cx, move |dialog, _, cx| {
            dialog
                .title(if delete {
                    "删除对话"
                } else {
                    "重命名对话"
                })
                .width(px(420.))
                .overlay_closable(false)
                .close_button(!editor.read(cx).busy)
                .keyboard(!editor.read(cx).busy)
                .child(editor.clone())
        });
        if !delete {
            window.focus(&title_focus, cx);
        }
    }
    fn mutate(&mut self, params: serde_json::Value, cx: &mut Context<Self>) {
        if self.mutating {
            return;
        }
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            self.error = Some("对话服务未连接。".into());
            cx.notify();
            return;
        };
        self.mutating = true;
        self.operation = Some(cx.spawn(async move |this, cx| {
            let result = engine.client().call("Mutate", params).await;
            let _ = this.update(cx, |this, cx| {
                this.mutating = false;
                match result {
                    Ok(_) => this.refresh(cx),
                    Err(error) => {
                        this.error = Some(error.to_string());
                        cx.notify();
                    }
                }
            });
        }));
        cx.notify();
    }
    fn batch(&mut self, ids: Vec<String>, action: serde_json::Value, cx: &mut Context<Self>) {
        if self.mutating || ids.is_empty() {
            return;
        }
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            self.error = Some("对话服务未连接。".into());
            cx.notify();
            return;
        };
        self.mutating = true;
        self.error = None;
        self.export_status = None;
        self.operation = Some(cx.spawn(async move |this, cx| {
            let result = engine
                .client()
                .call(
                    "Mutate",
                    serde_json::json!({"op":"batchLibrary", "chatIds":ids, "action":action}),
                )
                .await;
            let _ = this.update(cx, |this, cx| {
                this.mutating = false;
                match result {
                    Ok(result) => {
                        let succeeded = result["succeeded"].as_array().cloned().unwrap_or_default();
                        for id in &succeeded {
                            if let Some(id) = id.as_str() {
                                this.selected.remove(id);
                            }
                        }
                        let failures = result["failed"].as_array().cloned().unwrap_or_default();
                        this.export_status = Some(crate::ui_format!("已完成 {} 个对话，失败 {} 个。", "Completed {} conversations; {} failed.",
                            succeeded.len(),
                            failures.len()
                        ));
                        this.refresh(cx);
                        if !failures.is_empty() {
                            this.error = Some(
                                failures
                                    .iter()
                                    .map(|item| {
                                        format!(
                                            "{}：{}",
                                            item["id"].as_str().unwrap_or_default(),
                                            item["error"].as_str().unwrap_or_default()
                                        )
                                    })
                                    .collect::<Vec<_>>()
                                    .join("\n"),
                            );
                        }
                    }
                    Err(error) => this.error = Some(error.to_string()),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }
    fn delete_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.mutating || self.selected.is_empty() {
            return;
        }
        let ids: Vec<_> = self.selected.iter().cloned().collect();
        let owner = cx.weak_entity();
        window.open_dialog(cx, move |dialog, _, _| {
            let owner = owner.clone();
            let ids = ids.clone();
            dialog.title(crate::i18n::text("删除所选对话？"))
                .child(crate::ui_format!("将永久删除 {} 个对话及其消息，无法撤销。正在回复的对话不会删除，失败项会保留选中。", "Permanently delete {} conversations and their messages? This cannot be undone. Running conversations will be skipped; failed items stay selected.", ids.len()))
                .footer(div().flex().justify_end().gap_2()
                    .child(Button::new("cancel-library-batch-delete").label(crate::i18n::text("取消")).on_click(|_, window, cx| window.close_dialog(cx)))
                    .child(Button::new("confirm-library-batch-delete").danger().label(crate::i18n::text("删除所选对话")).on_click(move |_, window, cx| {
                        let _ = owner.update(cx, |this, cx| this.batch(ids.clone(), serde_json::json!({"op":"deleteChat"}), cx));
                        window.close_dialog(cx);
                    })))
        });
    }
    fn export_selected(&mut self, cx: &mut Context<Self>) {
        if self.mutating || self.selected.is_empty() {
            return;
        }
        let ids: Vec<_> = self.selected.iter().cloned().collect();
        let root = self.directory.clone();
        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("选择 Markdown 导出目录".into()),
        });
        self.mutating = true;
        self.error = None;
        self.operation = Some(cx.spawn(async move |this, cx| {
            let result = match prompt.await {
                Ok(Ok(Some(paths))) if !paths.is_empty() => {
                    let destination = paths[0].clone();
                    cx.background_spawn(async move {
                        use std::io::Write;
                        let store = ConversationStore::new(root);
                        let mut succeeded = Vec::new();
                        let mut failures = Vec::new();
                        for id in ids {
                            let result = (|| {
                                let conversation = store.load(&id).map_err(|e| e.to_string())?;
                                let path = destination.join(format!("{id}.md"));
                                let markdown = kivio_model_runtime::conversation_export::render_conversation_markdown(&conversation, "zh");
                                let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&path).map_err(|e| e.to_string())?;
                                if let Err(error) = file.write_all(markdown.as_bytes()).and_then(|_| file.sync_all()) {
                                    let _ = std::fs::remove_file(&path);
                                    return Err(error.to_string());
                                }
                                Ok::<_, String>(())
                            })();
                            match result { Ok(()) => succeeded.push(id), Err(error) => failures.push(format!("{id}：{error}")) }
                        }
                        Ok(Some((succeeded, failures)))
                    }).await
                }
                Ok(Ok(_)) => Ok(None),
                Ok(Err(error)) => Err(error.to_string()),
                Err(error) => Err(error.to_string()),
            };
            let _ = this.update(cx, |this, cx| {
                this.mutating = false;
                match result {
                    Ok(Some((succeeded, failures))) => {
                        for id in &succeeded { this.selected.remove(id); }
                        this.export_status = Some(crate::ui_format!("已导出 {} 个对话，失败 {} 个。已有同名文件不会覆盖。", "Exported {} conversations; {} failed. Existing files were not overwritten.", succeeded.len(), failures.len()));
                        if !failures.is_empty() { this.error = Some(failures.join("\n")); }
                    }
                    Ok(None) => {},
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }
    fn move_menu(&self, id: SharedString, ids: Vec<String>, cx: &Context<Self>) -> AnyElement {
        let groups = self.groups.clone();
        let owner = cx.weak_entity();
        Button::new(id).small().label(crate::i18n::text("移动到…")).disabled(self.mutating || ids.is_empty()).dropdown_menu(move |mut menu, _, _| {
            for (kind, group_id, label) in std::iter::once(("none", String::new(), "移出集与项目".to_string())).chain(groups.iter().map(|(set, id, label)| (if *set { "set" } else { "project" }, id.clone(), label.clone()))) {
                let owner = owner.clone();
                let ids = ids.clone();
                menu = menu.item(PopupMenuItem::new(label).on_click(move |_, _, cx| {
                    let _ = owner.update(cx, |this, cx| this.batch(ids.clone(), serde_json::json!({"op":"moveChat", "groupKind":kind, "groupId":group_id}), cx));
                }));
            }
            menu
        }).into_any_element()
    }
    fn transfer_set(
        &mut self,
        set_id: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.mutating {
            return;
        }
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            self.error = Some("对话服务未连接。".into());
            cx.notify();
            return;
        };
        let export_prompt = set_id
            .as_ref()
            .map(|_| cx.prompt_for_new_path(&self.directory, Some("Kivio-set.json")));
        let import_prompt = set_id.is_none().then(|| {
            cx.prompt_for_paths(PathPromptOptions {
                files: true,
                directories: false,
                multiple: false,
                prompt: Some("导入 Kivio 集备份（含对话与附件）".into()),
            })
        });
        self.mutating = true;
        self.error = None;
        self.export_status = None;
        self.operation = Some(cx.spawn_in(window, async move |this, cx| {
            let result = async {
                let path = if let Some(prompt) = export_prompt {
                    prompt
                        .await
                        .map_err(|e| e.to_string())?
                        .map_err(|e| e.to_string())?
                } else if let Some(prompt) = import_prompt {
                    prompt
                        .await
                        .map_err(|e| e.to_string())?
                        .map_err(|e| e.to_string())?
                        .and_then(|paths| paths.into_iter().next())
                } else {
                    None
                };
                let Some(path) = path else {
                    return Ok(None);
                };
                if let Some(id) = set_id {
                    engine
                        .client()
                        .call(
                            "KivioExportSet",
                            serde_json::json!({"setId":id,"path":path}),
                        )
                        .await
                        .map_err(|e| e.to_string())?;
                    Ok(Some((crate::ui_format!("已导出集备份到 {}", "Set backup exported to {}", path.display()), None)))
                } else {
                    let set = engine
                        .client()
                        .call("KivioImportSet", serde_json::json!({"path":path}))
                        .await
                        .map_err(|e| e.to_string())?;
                    Ok(Some((
                        crate::ui_format!("已导入集“{}”，原有数据保持不变。", "Imported set “{}”. Existing data was preserved.",
                            set["name"].as_str().unwrap_or_default()
                        ),
                        set["id"].as_str().map(str::to_owned),
                    )))
                }
            }
            .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.mutating = false;
                match result {
                    Ok(Some((status, imported))) => {
                        this.export_status = Some(status);
                        if let Some(id) = imported {
                            this.group = Some((true, id));
                            this.shelf = "all";
                            this.runtime_kind = "";
                            this.page = 0;
                            this.search
                                .update(cx, |search, cx| search.set_value("", window, cx));
                        }
                        this.refresh(cx);
                    }
                    Ok(None) => {}
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }
    fn export(&mut self, id: String, cx: &mut Context<Self>) {
        if self.mutating {
            return;
        }
        let prompt = cx.prompt_for_new_path(&self.directory, Some("Kivio-conversation.md"));
        let root = self.directory.clone();
        self.mutating = true;
        self.error = None;
        self.export_status = None;
        self.operation = Some(cx.spawn(async move |this, cx| {
            let result = match prompt.await {
                Ok(Ok(Some(path))) => {
                    cx.background_spawn(async move {
                        let conversation = ConversationStore::new(root)
                            .load(&id)
                            .map_err(|e| e.to_string())?;
                        let markdown =
                            kivio_model_runtime::conversation_export::render_conversation_markdown(
                                &conversation,
                                "zh",
                            );
                        std::fs::write(&path, markdown).map_err(|e| crate::ui_format!("导出失败：{e}", "Export failed: {e}"))?;
                        Ok(Some(crate::ui_format!("已导出到 {}", "Exported to {}", path.display())))
                    })
                    .await
                }
                Ok(Ok(None)) => Ok(None),
                Ok(Err(error)) => Err(error.to_string()),
                Err(error) => Err(error.to_string()),
            };
            let _ = this.update(cx, |this, cx| {
                this.mutating = false;
                match result {
                    Ok(status) => this.export_status = status,
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }
    fn refresh(&mut self, cx: &mut Context<Self>) {
        let directory = self.directory.clone();
        let query = ConversationLibraryQuery {
            offset: self.page * 40,
            limit: 40,
            shelf: self.shelf.into(),
            sort: self.sort.into(),
            order: if self.ascending { "asc" } else { "desc" }.into(),
            runtime_kind: Some(self.runtime_kind.into()),
            set_id: self
                .group
                .as_ref()
                .filter(|(set, _)| *set)
                .map(|(_, id)| id.clone()),
            project_id: self
                .group
                .as_ref()
                .filter(|(set, _)| !*set)
                .map(|(_, id)| id.clone()),
            q: Some(self.search.read(cx).value().to_string()),
            full_text: self.full_text,
            ..Default::default()
        };
        self.busy = true;
        self.error = None;
        self.task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let mut groups = Vec::new();
                    for set in kivio_model_runtime::group_catalog::load_set_index(&directory)?.sets
                    {
                        groups.push((true, set.id, crate::ui_format!("集 · {}", "Set · {}", set.name)));
                    }
                    for project in
                        kivio_model_runtime::group_catalog::load_project_index(&directory)?.projects
                    {
                        groups.push((false, project.id, crate::ui_format!("项目 · {}", "Project · {}", project.name)));
                    }
                    let conversations = ConversationStore::new(directory)
                        .load_all()
                        .map_err(|error| error.to_string())?;
                    let items = conversations
                        .iter()
                        .map(ConversationListItem::from)
                        .collect();
                    let by_id: std::collections::HashMap<_, _> =
                        conversations.iter().map(|c| (c.id.as_str(), c)).collect();
                    let page = conversation_library::query_items(
                        items,
                        query,
                        |id, needle| {
                            by_id.get(id).is_some_and(|c| {
                                conversation_search::first_message_match(c, needle).is_some()
                            })
                        },
                        |item, needle| {
                            let (field, id, snippet) = conversation_search::first_message_match(
                                by_id.get(item.id.as_str())?,
                                needle,
                            )?;
                            Some(ConversationSearchHit {
                                item,
                                match_field: field,
                                match_message_id: Some(id),
                                match_snippet: Some(snippet),
                            })
                        },
                    )?;
                    Ok::<_, String>((page, groups))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                match result {
                    Ok((page, groups)) => {
                        this.groups = groups;
                        if page.items.is_empty() && this.page > 0 && page.total <= this.page * 40 {
                            this.page = page.total.saturating_sub(1) / 40;
                            this.refresh(cx);
                            return;
                        }
                        this.rows = page.items;
                        this.total = page.total;
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }
}
impl Render for LibraryPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let english = crate::i18n::is_english();
        if self.english_placeholders != english {
            self.english_placeholders = english;
            self.search.update(cx, |input, cx| input.set_placeholder(crate::i18n::text("搜索标题、预览和正文"), window, cx));
        }
        let mut shelves = div().flex().flex_wrap().items_center().gap_1();
        for (id, label) in [
            ("all", "全部"),
            ("starred", "收藏"),
            ("uncategorized", "未归类"),
            ("recent7d", "近 7 天"),
            ("archived", "归档"),
        ] {
            shelves = shelves.child(
                Button::new(id)
                    .small()
                    .ghost()
                    .label(crate::i18n::display(label))
                    .selected(self.shelf == id)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.shelf = id;
                        this.page = 0;
                        this.refresh(cx);
                    })),
            );
        }
        let mut rows = div().v_flex();
        for hit in &self.rows {
            let item = &hit.item;
            let id = item.id.clone();
            let menu_id = id.clone();
            let selection_id = id.clone();
            let title = item.title.clone();
            let owner = cx.weak_entity();
            let pinned = item.pinned;
            let archived = item.archived;
            let date = chrono::DateTime::from_timestamp(item.updated_at, 0)
                .map(|d| {
                    d.with_timezone(&chrono::Local)
                        .format("%Y-%m-%d %H:%M")
                        .to_string()
                })
                .unwrap_or_default();
            rows = rows.child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .py_2()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(Checkbox::new(SharedString::from(format!("library-select-{selection_id}")))
                        .checked(self.selected.contains(&selection_id))
                        .accessibility_label(crate::ui_format!("选择对话 {}", "Select conversation {}", item.title))
                        .disabled(self.mutating)
                        .on_click(cx.listener(move |this, checked, _, cx| {
                            if *checked { this.selected.insert(selection_id.clone()); } else { this.selected.remove(&selection_id); }
                            cx.notify();
                        })))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div().flex().min_w_0().child(
                                    Button::new(SharedString::from(format!("library-open-{id}")))
                                        .small()
                                        .ghost()
                                        .max_w_full()
                                        .label(item.title.clone())
                                        .on_click(cx.listener(move |_, _, _, cx| {
                                            cx.emit(OpenConversation(id.clone()))
                                        })),
                                ),
                            )
                            .child(
                                div()
                                    .px_3()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .truncate()
                                    .child(
                                        hit.match_snippet
                                            .as_deref()
                                            .unwrap_or(&item.preview)
                                            .split_whitespace()
                                            .collect::<Vec<_>>()
                                            .join(" "),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .w(px(128.))
                            .text_size(px(12.))
                            .text_color(cx.theme().muted_foreground)
                            .child(date),
                    )
                    .child(self.move_menu(SharedString::from(format!("library-move-{menu_id}")), vec![menu_id.clone()], cx))
                    .child(
                        Button::new(SharedString::from(format!("library-more-{menu_id}")))
                            .small().ghost().label(crate::i18n::text("更多…"))
                            .disabled(self.busy || self.mutating)
                            .dropdown_menu(move |mut menu, _, _| {
                                for (action, label) in [
                                    ("rename", "重命名…"),
                                    ("pin", if pinned { "取消收藏" } else { "收藏" }),
                                    ("archive", if archived { "恢复对话" } else { "归档" }),
                                    ("export", "导出 Markdown…"),
                                    ("delete", "删除对话…"),
                                ] {
                                    if action == "delete" { menu = menu.separator(); }
                                    let owner = owner.clone();
                                    let id = menu_id.clone();
                                    let title = title.clone();
                                    menu = menu.item(PopupMenuItem::new(label).on_click(move |_, window, cx| {
                                        let _ = owner.update(cx, |this, cx| match action {
                                            "rename" | "delete" => this.edit(id.clone(), title.clone(), action == "delete", window, cx),
                                            "export" => this.export(id.clone(), cx),
                                            "pin" => this.mutate(serde_json::json!({"op":"setChatPinned", "chatId":id, "pinned":!pinned}), cx),
                                            _ => this.mutate(serde_json::json!({"op":"setChatArchived", "chatId":id, "archived":!archived}), cx),
                                        });
                                    }));
                                }
                                menu
                            }),
                    ),
            );
        }
        if self.rows.is_empty() && !self.busy {
            rows = rows.child(
                div()
                    .py_12()
                    .text_center()
                    .text_color(cx.theme().muted_foreground)
                    .child(crate::i18n::text("没有匹配的对话。换个书架或清空搜索条件试试。")),
            );
        }
        let owner = cx.weak_entity();
        let sort = self.sort;
        div()
            .v_flex()
            .size_full()
            .min_h_0()
            .max_w(px(1000.))
            .mx_auto()
            .px(px(40.))
            .pt(px(crate::theme::Theme::TITLEBAR_HEIGHT + 16.))
            .pb_6()
            .gap_4()
            .text_size(crate::typography::ui_rems(13.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_size(crate::typography::ui_rems(20.))
                            .font_weight(FontWeight::MEDIUM)
                            .child(crate::i18n::text("对话库")),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(crate::ui_format!("{} 个对话", "{} conversations", self.total))
                            .child(Button::new("library-import-set").small().label(crate::i18n::text("导入集…")).disabled(self.mutating).on_click(cx.listener(|this, _, window, cx| this.transfer_set(None, window, cx))))
                            .when_some(self.group.as_ref().filter(|(set, _)| *set).map(|(_, id)| id.clone()), |view, id| {
                                view.child(Button::new("library-export-set").small().label(crate::i18n::text("导出此集…")).disabled(self.mutating).on_click(cx.listener(move |this, _, window, cx| this.transfer_set(Some(id.clone()), window, cx))))
                            })
                            .child(
                                Button::new("library-refresh")
                                    .small()
                                    .label(if self.busy { crate::i18n::text("读取中…") } else { crate::i18n::text("刷新") })
                                    .disabled(self.busy)
                                    .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))),
                            ),
                    ),
            )
            .when_some(self.export_status.clone(), |view, status| {
                view.child(div().text_sm().child(status))
            })
            .when_some(self.error.clone(), |view, error| {
                view.child(div().text_color(cx.theme().danger).child(error))
            })
            .child(
                div()
                    .v_flex()
                    .flex_1()
                    .min_h_0()
                    .bg(cx.theme().group_box)
                    .rounded(px(12.))
                    .child(
                        div()
                            .flex_none()
                            .p_3()
                            .border_b_1()
                            .border_color(cx.theme().border)
                            .child(shelves),
                    )
                    .child(
                        div()
                            .v_flex()
                            .flex_1()
                            .min_w_0()
                            .min_h_0()
                            .p_3()
                            .gap_3()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_3()
                                    .child(div().flex_1().min_w_0().child(
                                        Input::new(&self.search).small().aria_label("搜索对话"),
                                    ))
                                    .child(
                                        Button::new("library-sort")
                                            .small()
                                            .label(match self.sort {
                                                "created" => "创建时间",
                                                "title" => "标题",
                                                "messages" => "消息数",
                                                _ => "最近更新",
                                            })
                                            .dropdown_caret(true)
                                            .dropdown_menu(move |mut menu, _, _| {
                                                for (id, label) in [
                                                    ("updated", "最近更新"),
                                                    ("created", "创建时间"),
                                                    ("title", "标题"),
                                                    ("messages", "消息数"),
                                                ] {
                                                    let owner = owner.clone();
                                                    menu = menu.item(
                                                        PopupMenuItem::new(label)
                                                            .checked(sort == id)
                                                            .on_click(move |_, _, cx| {
                                                                let _ =
                                                                    owner.update(cx, |this, cx| {
                                                                        this.sort = id;
                                                                        this.page = 0;
                                                                        this.refresh(cx);
                                                                    });
                                                            }),
                                                    );
                                                }
                                                menu
                                            }),
                                    )
                                    .child(
                                        Button::new("library-order")
                                            .small()
                                            .label(if self.ascending { crate::i18n::text("升序") } else { crate::i18n::text("降序") })
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.ascending = !this.ascending;
                                                this.page = 0;
                                                this.refresh(cx);
                                            })),
                                    )
                                    .child(
                                        Switch::new("library-full-text")
                                            .label(crate::i18n::text("搜正文"))
                                            .checked(self.full_text)
                                            .on_click(cx.listener(|this, checked, _, cx| {
                                                this.full_text = *checked;
                                                this.page = 0;
                                                this.refresh(cx);
                                            })),
                                    ),
                            )
                            .child({
                                let group_owner = cx.weak_entity();
                                let groups = self.groups.clone();
                                let selected = self.group.clone();
                                let label = self
                                    .groups
                                    .iter()
                                    .find(|(set, id, _)| {
                                        self.group.as_ref().is_some_and(
                                            |(current_set, current_id)| {
                                                current_set == set && current_id == id
                                            },
                                        )
                                    })
                                    .map(|(_, _, name)| name.clone())
                                    .unwrap_or_else(|| "全部集与项目".into());
                                let mut choices = div().flex().flex_wrap().gap_2().child(
                                    Button::new("library-group")
                                        .small()
                                        .label(crate::i18n::display(label))
                                        .dropdown_caret(true)
                                        .dropdown_menu(move |mut menu, _, _| {
                                            let owner = group_owner.clone();
                                            menu = menu.item(
                                                PopupMenuItem::new(crate::i18n::text("全部集与项目"))
                                                    .checked(selected.is_none())
                                                    .on_click(move |_, _, cx| {
                                                        let _ = owner.update(cx, |this, cx| {
                                                            this.group = None;
                                                            this.page = 0;
                                                            this.refresh(cx);
                                                        });
                                                    }),
                                            );
                                            for (set, id, label) in &groups {
                                                let owner = group_owner.clone();
                                                let value = (*set, id.clone());
                                                let value_for_click = value.clone();
                                                menu = menu.item(
                                                    PopupMenuItem::new(label.clone())
                                                        .checked(selected.as_ref() == Some(&value))
                                                        .on_click(move |_, _, cx| {
                                                            let _ = owner.update(cx, |this, cx| {
                                                                this.group =
                                                                    Some(value_for_click.clone());
                                                                this.page = 0;
                                                                this.refresh(cx);
                                                            });
                                                        }),
                                                );
                                            }
                                            menu
                                        }),
                                );
                                for (kind, label) in [
                                    ("", "全部运行方式"),
                                    ("builtin", "内置模型"),
                                    ("external", "外部 CLI"),
                                ] {
                                    choices =
                                        choices.child(
                                            Button::new(SharedString::from(format!(
                                                "library-runtime-{kind}"
                                            )))
                                            .small()
                                            .ghost()
                                            .label(crate::i18n::display(label))
                                            .selected(self.runtime_kind == kind)
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.runtime_kind = kind;
                                                this.page = 0;
                                                this.refresh(cx);
                                            })),
                                        );
                                }
                                choices
                            })
                            .child({
                                let ids: Vec<_> = self.selected.iter().cloned().collect();
                                let mut toolbar = div().flex().flex_wrap().items_center().gap_2()
                                    .child(Button::new("library-select-page").small().ghost().label(crate::i18n::text("选择本页")).disabled(self.busy || self.mutating).on_click(cx.listener(|this, _, _, cx| { this.selected.extend(this.rows.iter().map(|hit| hit.item.id.clone())); cx.notify(); })))
                                    .child(Button::new("library-clear-selection").small().ghost().label(crate::ui_format!("清除选择 ({})", "Clear selection ({})", ids.len())).disabled(self.mutating || ids.is_empty()).on_click(cx.listener(|this, _, _, cx| { this.selected.clear(); cx.notify(); })));
                                if !ids.is_empty() {
                                    toolbar = toolbar.child(self.move_menu("library-bulk-move".into(), ids.clone(), cx));
                                    for (key, label, action) in [
                                        ("pin", "收藏", serde_json::json!({"op":"setChatPinned","pinned":true})),
                                        ("unpin", "取消收藏", serde_json::json!({"op":"setChatPinned","pinned":false})),
                                        ("archive", "归档", serde_json::json!({"op":"setChatArchived","archived":true})),
                                        ("restore", "恢复", serde_json::json!({"op":"setChatArchived","archived":false})),
                                    ] {
                                        let ids = ids.clone();
                                        toolbar = toolbar.child(Button::new(SharedString::from(format!("library-bulk-{key}"))).small().label(crate::i18n::display(label)).disabled(self.mutating).on_click(cx.listener(move |this, _, _, cx| this.batch(ids.clone(), action.clone(), cx))));
                                    }
                                    toolbar = toolbar
                                        .child(Button::new("library-bulk-export").small().label(crate::i18n::text("导出 Markdown…")).disabled(self.mutating).on_click(cx.listener(|this, _, _, cx| this.export_selected(cx))))
                                        .child(Button::new("library-bulk-delete").small().danger().label(crate::i18n::text("删除…")).disabled(self.mutating).on_click(cx.listener(|this, _, window, cx| this.delete_selected(window, cx))));
                                }
                                toolbar
                            })
                            .child(
                                div()
                                    .flex()
                                    .justify_between()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(crate::i18n::text("标题 / 内容预览"))
                                    .child(crate::i18n::text("更新时间")),
                            )
                            .child(
                                div()
                                    .id("library-rows")
                                    .flex_1()
                                    .min_h_0()
                                    .overflow_y_scroll()
                                    .child(rows),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .child(
                                        Button::new("library-previous")
                                            .small()
                                            .label(crate::i18n::text("上一页"))
                                            .disabled(self.busy || self.page == 0)
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.page = this.page.saturating_sub(1);
                                                this.refresh(cx);
                                            })),
                                    )
                                    .child(format!(
                                        "{} / {}",
                                        self.page + 1,
                                        self.total.div_ceil(40).max(1)
                                    ))
                                    .child(
                                        Button::new("library-next")
                                            .small()
                                            .label(crate::i18n::text("下一页"))
                                            .disabled(
                                                self.busy || (self.page + 1) * 40 >= self.total,
                                            )
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.page += 1;
                                                this.refresh(cx);
                                            })),
                                    ),
                            ),
                    ),
            )
    }
}

/// The dialog retains its draft and service error until a successful mutation.
struct ConversationEditor {
    owner: WeakEntity<LibraryPage>,
    id: String,
    title: Entity<InputState>,
    original: String,
    delete: bool,
    busy: bool,
    error: Option<String>,
    task: Option<Task<()>>,
}
impl ConversationEditor {
    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let title = self.title.read(cx).value().trim().to_owned();
        if !self.delete && title.is_empty() {
            self.error = Some("请输入对话标题。".into());
            cx.notify();
            return;
        }
        let Some(owner) = self.owner.upgrade() else {
            return;
        };
        if owner.read(cx).mutating {
            return;
        }
        let Some(engine) = owner.read(cx).state.read(cx).engine().cloned() else {
            self.error = Some("对话服务未连接。".into());
            cx.notify();
            return;
        };
        let params = if self.delete {
            serde_json::json!({"op":"deleteChat", "chatId":self.id})
        } else {
            serde_json::json!({"op":"renameChat", "chatId":self.id, "title":title})
        };
        owner.update(cx, |owner, cx| {
            owner.mutating = true;
            cx.notify();
        });
        self.busy = true;
        self.error = None;
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = engine.client().call("Mutate", params).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                owner.update(cx, |owner, cx| {
                    owner.mutating = false;
                    if result.is_ok() {
                        owner.refresh(cx);
                    }
                    cx.notify();
                });
                match result {
                    Ok(_) => window.close_dialog(cx),
                    Err(error) => this.error = Some(error.to_string()),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }
}
impl Render for ConversationEditor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .v_flex()
            .gap_4()
            .when(self.delete, |view| {
                view.child(crate::ui_format!("确定删除“{}”及其全部消息？此操作无法撤销。正在生成回复的对话需先停止回复。", "Delete “{}” and all its messages? This cannot be undone. Stop an active reply before deleting its conversation.",
                    self.original
                ))
            })
            .when(!self.delete, |view| {
                view.child(
                    Input::new(&self.title)
                        .disabled(self.busy)
                        .aria_label("对话标题"),
                )
            })
            .when_some(self.error.clone(), |view, error| {
                view.child(div().text_sm().text_color(cx.theme().danger).child(error))
            })
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("library-dialog-cancel")
                            .label(crate::i18n::text("取消"))
                            .disabled(self.busy)
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("library-dialog-confirm")
                            .label(if self.busy { crate::i18n::text("处理中…") } else if self.delete { crate::i18n::text("删除对话") } else { crate::i18n::text("保存") })
                            .when(self.delete, |button| button.danger())
                            .when(!self.delete, |button| button.primary())
                            .disabled(self.busy)
                            .on_click(cx.listener(|this, _, window, cx| this.submit(window, cx))),
                    ),
            )
    }
}
