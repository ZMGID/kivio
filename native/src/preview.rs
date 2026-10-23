use gpui_kit::component::{
    ActiveTheme, Disableable, Icon, RopeExt, Sizable, StyledExt, WindowExt,
    button::{Button, ButtonVariants},
    input::{InputEvent, MoveDown, MoveUp, Textarea, TextareaState},
    menu::{DropdownMenu, PopupMenuItem},
};
use gpui_kit::{assets::IconName, prelude::FluentBuilder, *};

use crate::transcript::{
    Changed, MessageAction, MessageActionRequested, RetryRequested, RewindRequested, Transcript,
};
use kivio_model_runtime::attachment_files::{self, AttachmentBatch};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::Arc,
};

pub enum ConversationEvent {
    FirstMessage(SharedString),
    StatusChanged,
    Submitted,
    DraftChanged,
    BindingChanged,
    ConfigureProviders,
    MessageAction(MessageActionRequested),
}

/// A retained conversation pane: switching conversations never reconstructs
/// its draft, scroll state, transcript or in-flight task.
pub struct Preview {
    transcript: Entity<Transcript>,
    momo: Entity<crate::momo::Momo>,
    fork_title: Option<SharedString>,
    input: Entity<TextareaState>,
    attachments: Vec<PathBuf>,
    thumbnails: HashMap<PathBuf, Arc<Image>>,
    clipboard_files: HashSet<PathBuf>,
    thumbnail_task: Option<Task<()>>,
    attachment_task: Option<Task<()>>,
    mutation_busy: bool,
    catalog_dir: Option<PathBuf>,
    attachment_error: Option<SharedString>,
    picker: Option<Task<()>>,
    history_only: bool,
    sample: bool,
    conversation_id: String,
    providers: Option<Entity<crate::providers::ProviderSettings>>,
    binding: Option<(String, String)>,
    history: Option<(Vec<SharedString>, usize, SharedString)>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<ConversationEvent> for Preview {}

impl Preview {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let transcript = cx.new(Transcript::new);
        transcript.update(cx, |view, cx| {
            view.watch_window(window, cx);
        });
        let input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(1, 8)
                .submit_on_enter(true)
                .placeholder("有问题尽管问…")
        });
        let subscriptions =
            vec![
                cx.on_release(|this, cx| {
                    for image in this.thumbnails.values() {
                        image.clone().remove_asset(cx);
                    }
                    for path in &this.clipboard_files {
                        let _ = std::fs::remove_file(path);
                    }
                }),
                cx.subscribe_in(&input, window, |this, _, event: &InputEvent, window, cx| {
                    match event {
                        InputEvent::PressEnter { shift: false, .. } => this.send(window, cx),
                        InputEvent::Change => {
                            if this.history.as_ref().is_some_and(|(entries, index, _)| {
                                this.input.read(cx).value() != entries[*index]
                            }) {
                                this.history = None;
                            }
                            cx.emit(ConversationEvent::DraftChanged);
                            cx.notify();
                        }
                        InputEvent::Focus | InputEvent::Blur => cx.notify(),
                        _ => {}
                    }
                }),
                cx.subscribe_in(
                    &transcript,
                    window,
                    |this, _, request: &RewindRequested, window, cx| {
                        if this.sample || this.catalog_dir.is_none() {
                            this.confirm_rewind(request.0, window, cx);
                        } else if let Some(message_id) =
                            this.transcript.read(cx).canonical_id(request.0)
                        {
                            cx.emit(ConversationEvent::MessageAction(MessageActionRequested {
                                message_id,
                                action: MessageAction::Rewind,
                            }));
                        }
                    },
                ),
                cx.subscribe_in(
                    &transcript,
                    window,
                    |this, _, _: &RetryRequested, window, cx| this.retry(window, cx),
                ),
                cx.subscribe(&transcript, |_, _, request: &MessageActionRequested, cx| {
                    cx.emit(ConversationEvent::MessageAction(request.clone()));
                }),
                cx.subscribe(&transcript, |_, _, _: &Changed, cx| {
                    cx.emit(ConversationEvent::StatusChanged);
                    cx.notify();
                }),
            ];
        input.update(cx, |state, cx| state.focus(window, cx));
        Self {
            transcript,
            momo: cx.new(|_| crate::momo::Momo::default()),
            fork_title: None,
            input,
            attachments: Vec::new(),
            thumbnails: HashMap::new(),
            clipboard_files: HashSet::new(),
            thumbnail_task: None,
            attachment_task: None,
            mutation_busy: false,
            catalog_dir: None,
            attachment_error: None,
            picker: None,
            history_only: false,
            sample: false,
            conversation_id: uuid::Uuid::new_v4().to_string(),
            providers: None,
            binding: None,
            history: None,
            _subscriptions: subscriptions,
        }
    }

    pub fn configure(
        &mut self,
        providers: Entity<crate::providers::ProviderSettings>,
        binding: Option<(String, String)>,
        conversation_id: String,
        catalog_dir: PathBuf,
    ) {
        self.catalog_dir = Some(catalog_dir);
        self.conversation_id = conversation_id;
        self.providers = Some(providers);
        self.binding = binding;
    }

    pub fn set_fork_origin(&mut self, origin: Option<&crate::conversation_data::ForkOrigin>) {
        self.fork_title = origin.map(|origin| origin.title.clone().into());
    }

    pub fn binding(&self) -> Option<&(String, String)> {
        self.binding.as_ref()
    }

    pub fn set_history_only(&mut self, value: bool) {
        self.history_only = value;
    }

    pub fn is_busy(&self, _cx: &App) -> bool {
        self.mutation_busy || self.attachment_task.is_some() || self.picker.is_some()
    }

    pub fn set_mutation_busy(&mut self, busy: bool, cx: &mut Context<Self>) {
        self.mutation_busy = busy;
        cx.notify();
    }

    pub fn stop(&mut self, cx: &mut Context<Self>) {
        self.attachment_task = None;
        self.picker = None;
        self.transcript.update(cx, |view, cx| view.stop(cx));
        cx.notify();
    }

    pub fn snapshot(&self, cx: &App) -> (Vec<crate::conversation_data::ChatMessage>, String) {
        (
            self.transcript.read(cx).snapshot(),
            self.input.read(cx).value().to_string(),
        )
    }

    pub fn restore(
        &mut self,
        messages: Vec<crate::conversation_data::ChatMessage>,
        draft: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.transcript.update(cx, |view, cx| {
            if let Some(directory) = &self.catalog_dir {
                view.set_attachment_directory(
                    directory.join(format!("{}_attachments", self.conversation_id)),
                );
            }
            view.restore(messages, cx);
        });
        self.input
            .update(cx, |input, cx| input.set_value(draft, window, cx));
    }

    pub fn focus_input(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.input.update(cx, |state, cx| state.focus(window, cx));
        cx.notify();
    }

    pub fn set_sample_mode(&mut self, sample: bool, cx: &mut Context<Self>) {
        self.sample = sample;
        self.transcript
            .update(cx, |view, _| view.set_sample_mode(sample));
    }

    pub fn load_sample(&mut self, count: usize, cx: &mut Context<Self>) {
        self.sample = true;
        self.fork_title = None;
        self.transcript.update(cx, |view, cx| view.load(count, cx));
    }

    pub fn reveal_message(&mut self, message_id: &str, cx: &mut Context<Self>) {
        self.transcript
            .update(cx, |view, cx| view.reveal_message(message_id, cx));
    }

    pub fn remeasure(&self, cx: &mut Context<Self>) {
        self.transcript.update(cx, |view, cx| view.remeasure(cx));
        // Retained entity views also cache their own layout. Invalidate the
        // composer when the shell changes its available width.
        cx.notify();
    }

    pub fn is_streaming(&self, cx: &App) -> bool {
        self.transcript.read(cx).is_streaming()
    }

    fn add_attachments(&mut self, paths: &[PathBuf], cx: &mut Context<Self>) {
        if self.mutation_busy || self.history_only {
            return;
        }
        for path in paths {
            if !self.attachments.contains(path) {
                self.attachments.push(path.clone());
            }
        }
        self.attachment_error = None;
        let paths = self.attachments.clone();
        let work = cx.background_spawn(async move {
            paths
                .into_iter()
                .filter_map(|path| {
                    attachment_files::attachment_thumbnail(&path).map(|bytes| (path, bytes))
                })
                .collect::<Vec<_>>()
        });
        self.thumbnail_task = Some(cx.spawn(async move |this, cx| {
            let thumbnails = work.await;
            let _ = this.update(cx, |this, cx| {
                for (path, bytes) in thumbnails {
                    if this.attachments.contains(&path) {
                        if let Some(old) = this
                            .thumbnails
                            .insert(path, Arc::new(Image::from_bytes(ImageFormat::Png, bytes)))
                        {
                            old.remove_asset(cx);
                        }
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn remove_attachment(&mut self, path: &PathBuf, cx: &mut Context<Self>) {
        self.attachments.retain(|current| current != path);
        if let Some(image) = self.thumbnails.remove(path) {
            image.remove_asset(cx);
        }
        if self.clipboard_files.remove(path) {
            let _ = std::fs::remove_file(path);
        }
        cx.notify();
    }

    fn paste_attachments(
        &mut self,
        _: &gpui_kit::component::input::Paste,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.mutation_busy || self.history_only {
            return;
        }
        let Some(clipboard) = cx.read_from_clipboard() else {
            return;
        };
        let mut paths = Vec::new();
        let mut images = Vec::new();
        for entry in clipboard.entries {
            match entry {
                ClipboardEntry::ExternalPaths(files) => paths.extend(files.paths().iter().cloned()),
                ClipboardEntry::Image(image) => images.push(image),
                ClipboardEntry::String(_) => {}
            }
        }
        if paths.is_empty() && images.is_empty() {
            return;
        }
        cx.stop_propagation();
        if !images.is_empty() {
            let work = cx.background_spawn(async move {
                let mut files = attachment_files::TemporaryAttachmentFiles::new();
                for image in images {
                    let (name, mime) = match image.format() {
                        ImageFormat::Png => ("粘贴图片.png", "image/png"),
                        ImageFormat::Jpeg => ("粘贴图片.jpg", "image/jpeg"),
                        ImageFormat::Gif => ("粘贴图片.gif", "image/gif"),
                        ImageFormat::Webp => ("粘贴图片.webp", "image/webp"),
                        ImageFormat::Bmp => ("粘贴图片.bmp", "image/bmp"),
                        ImageFormat::Tiff => ("粘贴图片.tiff", "image/tiff"),
                        _ => {
                            return Err(
                                "不支持此剪贴板图片格式，请保存为 PNG 或 JPEG 后添加。".to_string()
                            );
                        }
                    };
                    files.push(attachment_files::save_clipboard_image(
                        name,
                        mime,
                        image.bytes(),
                    )?);
                }
                Ok::<_, String>(files)
            });
            // A clipboard write belongs to this pane; it never captures another
            // conversation's input or model selection.
            self.picker = Some(cx.spawn(async move |this, cx| {
                let result = work.await;
                let _ = this.update(cx, |this, cx| {
                    this.picker = None;
                    match result {
                        Ok(saved) => {
                            let saved = saved.commit();
                            this.clipboard_files.extend(saved.iter().cloned());
                            this.add_attachments(&saved, cx);
                        }
                        Err(error) => this.attachment_error = Some(error.into()),
                    }
                    cx.notify();
                });
            }));
        }
        self.add_attachments(&paths, cx);
    }

    fn input_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.input.read(cx).focus_handle(cx).is_focused(window) {
            return;
        }
        if self.input.update(cx, |input, cx| {
            input.marked_text_range(window, cx).is_some()
        }) {
            return;
        }
        let key = &event.keystroke;
        if key.key == "escape" && (self.is_streaming(cx) || self.attachment_task.is_some()) {
            self.stop(cx);
            cx.stop_propagation();
        }
    }

    fn browse_history(&mut self, up: bool, window: &mut Window, cx: &mut Context<Self>) {
        if !self.input.read(cx).focus_handle(cx).is_focused(window)
            || self.input.update(cx, |input, cx| {
                input.marked_text_range(window, cx).is_some()
            })
        {
            return;
        }
        let input = self.input.read(cx);
        let value = input.value();
        let selection = input.selected_range();
        if !selection.is_empty()
            || (value.contains('\n')
                && if up {
                    selection.start != 0
                } else {
                    selection.end != value.len()
                })
        {
            return;
        }
        if self.history.is_none() && up {
            let entries = self.transcript.read(cx).input_history();
            if entries.is_empty() {
                return;
            }
            let index = entries.len();
            self.history = Some((entries, index, value));
        }
        let Some((entries, index, draft)) = self.history.as_mut() else {
            return;
        };
        *index = if up {
            index.saturating_sub(1)
        } else {
            (*index + 1).min(entries.len())
        };
        let next = entries.get(*index).unwrap_or(draft).clone();
        if *index == entries.len() {
            self.history = None;
        }
        self.input.update(cx, |input, cx| {
            input.set_value(next, window, cx);
            let end = input.text().offset_to_position(input.value().len());
            input.set_cursor_position(end, window, cx);
        });
        cx.stop_propagation();
        cx.notify();
    }

    fn rewind(&mut self, id: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(body) = self.transcript.update(cx, |view, cx| view.rewind(id, cx)) {
            self.history = None;
            self.input.update(cx, |input, cx| {
                input.set_value(body, window, cx);
                let end = input.text().offset_to_position(input.value().len());
                input.set_cursor_position(end, window, cx);
            });
        }
        window.close_dialog(cx);
    }

    fn confirm_rewind(&self, id: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.history_only {
            window.push_notification(
                gpui_kit::component::notification::Notification::error(
                    "此会话的运行时尚未接入，暂不能回退历史。",
                ),
                cx,
            );
            return;
        }
        let owner = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, _, _| {
            let keyboard = owner.clone();
            let confirm = owner.clone();
            dialog
                .title("回到这条提问？")
                .child(
                    "将移除这条提问及之后的回复，并把提问放回输入框。当前未发送的草稿也会被替换。",
                )
                .on_ok(move |_, window, cx| {
                    let _ = keyboard.update(cx, |this, cx| this.rewind(id, window, cx));
                    false
                })
                .footer(
                    div()
                        .flex()
                        .justify_end()
                        .gap_2()
                        .child(
                            Button::new("cancel-rewind")
                                .ghost()
                                .label("取消")
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(
                            Button::new("confirm-rewind")
                                .primary()
                                .label("回到这里")
                                .on_click(move |_, window, cx| {
                                    let _ =
                                        confirm.update(cx, |this, cx| this.rewind(id, window, cx));
                                }),
                        ),
                )
        });
    }

    fn choose_attachments(&mut self, cx: &mut Context<Self>) {
        if self.is_busy(cx) || self.history_only {
            return;
        }
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: true,
            multiple: true,
            prompt: Some("添加附件".into()),
        });
        self.picker = Some(cx.spawn(async move |this, cx| {
            let result = rx.await;
            let _ = this.update(cx, |this, cx| {
                this.picker = None;
                match result {
                    Ok(Ok(Some(paths))) => {
                        this.add_attachments(&paths, cx);
                    }
                    Ok(Ok(None)) => {}
                    _ => this.attachment_error = Some("无法打开文件选择器，请重试。".into()),
                }
                cx.notify();
            });
        }));
    }

    fn send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_busy(cx) || self.is_streaming(cx) {
            return;
        }
        if self.history_only {
            self.attachment_error = Some("此会话的运行时尚未接入，草稿已保留。".into());
            cx.notify();
            return;
        }
        let body = self.input.read(cx).value();
        if body.trim().is_empty() && self.attachments.is_empty() {
            return;
        }
        if self.sample {
            if !self.attachments.is_empty() {
                self.attachment_error = Some("开发样本不发送附件，请新建对话并选择模型。".into());
                cx.notify();
                return;
            }
            let first = self.transcript.read(cx).count() == 0;
            if self
                .transcript
                .update(cx, |view, cx| view.send_sample(body.clone(), cx))
            {
                self.did_submit(body.clone(), body, first, window, cx);
            }
            return;
        }
        let selection = self
            .providers
            .as_ref()
            .ok_or_else(|| "请先配置模型供应商。".to_owned())
            .and_then(|providers| {
                let settings = providers.read(cx);
                match &self.binding {
                    Some((provider, model)) => settings.selected(provider, model),
                    None => settings.selection(),
                }
            });
        let (provider, model) = match selection {
            Ok(value) => value,
            Err(error) => {
                self.attachment_error = Some(error.into());
                cx.notify();
                return;
            }
        };
        let Some(directory) = self.catalog_dir.clone() else {
            self.attachment_error = Some("缺少对话数据目录，草稿已保留。".into());
            cx.notify();
            return;
        };
        if self.attachments.is_empty() {
            self.submit(body, Vec::new(), provider, model, directory, window, cx);
            return;
        }
        let source_paths = self.attachments.clone();
        let attachment_dir = directory.join(format!("{}_attachments", self.conversation_id));
        let paths = source_paths
            .iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect();
        let work = cx.background_spawn(async move {
            let batch = AttachmentBatch::prepare(attachment_dir.clone(), paths)?;
            attachment_files::validate_attachment_media(&attachment_dir, batch.attachments())?;
            Ok::<_, String>(batch)
        });
        self.attachment_error = None;
        self.attachment_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = work.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.attachment_task = None;
                match result {
                    Ok(batch) => {
                        if this.submit(
                            body,
                            batch.attachments().to_vec(),
                            provider,
                            model,
                            directory,
                            window,
                            cx,
                        ) {
                            batch.commit();
                            for path in source_paths {
                                this.remove_attachment(&path, cx);
                            }
                        }
                    }
                    Err(error) => this.attachment_error = Some(error.into()),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn submit(
        &mut self,
        body: SharedString,
        attachments: Vec<crate::conversation_data::Attachment>,
        provider: crate::provider_settings::ModelProvider,
        model: String,
        directory: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let first = self.transcript.read(cx).count() == 0;
        let title = attachment_files::title_source_for_user_message(&body, &attachments).into();
        let binding = (provider.id.clone(), model.clone());
        match self.transcript.update(cx, |view, cx| {
            view.send(
                body.clone(),
                attachments,
                provider,
                model,
                self.conversation_id.clone(),
                directory,
                cx,
            )
        }) {
            Ok(true) => {
                self.binding = Some(binding);
                self.attachment_error = None;
                self.did_submit(body, title, first, window, cx);
                true
            }
            Ok(false) => false,
            Err(error) => {
                self.attachment_error = Some(error.into());
                cx.notify();
                false
            }
        }
    }

    fn did_submit(
        &mut self,
        body: SharedString,
        title: SharedString,
        first: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if first {
            cx.emit(ConversationEvent::FirstMessage(title));
        } else {
            cx.emit(ConversationEvent::Submitted);
        }
        self.history = None;
        self.input.update(cx, |state, cx| {
            if state.value() == body {
                state.set_value("", window, cx);
            }
            state.focus(window, cx);
        });
    }

    fn retry(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.history_only || self.sample || self.is_busy(cx) {
            return;
        }
        let result = self
            .providers
            .as_ref()
            .ok_or_else(|| "请先配置模型供应商。".to_owned())
            .and_then(|providers| {
                let settings = providers.read(cx);
                match &self.binding {
                    Some((provider, model)) => settings.selected(provider, model),
                    None => settings.selection(),
                }
            })
            .and_then(|(provider, model)| {
                self.transcript.update(cx, |view, cx| {
                    view.retry(
                        provider,
                        model,
                        self.conversation_id.clone(),
                        self.catalog_dir
                            .clone()
                            .ok_or_else(|| "缺少对话数据目录".to_owned())?,
                        cx,
                    )
                })
            });
        if let Err(error) = result {
            window.push_notification(
                gpui_kit::component::notification::Notification::error(error),
                cx,
            );
        }
    }

    // Zeron composer.rs: model selector belongs to the input action row,
    // beside Send, and follows this retained conversation's binding.
    fn model_picker(&self, cx: &mut Context<Self>) -> AnyElement {
        let choices = self
            .providers
            .as_ref()
            .map(|p| p.read(cx).models())
            .unwrap_or_default();
        let chosen = self.binding.clone();
        let label = chosen
            .as_ref()
            .map(|(_, model)| model.clone())
            .filter(|model| !model.is_empty())
            .unwrap_or_else(|| "选择模型".into());
        let busy = self.is_busy(cx) || self.is_streaming(cx) || self.history_only;
        let owner = cx.weak_entity();
        Button::new("composer-model")
            .small()
            .ghost()
            .label(label)
            .dropdown_caret(true)
            .disabled(busy)
            .accessibility_label("选择模型")
            .dropdown_menu(move |mut menu, _, _| {
                for choice in &choices {
                    let provider = choice.provider_id.clone();
                    let model = choice.model.clone();
                    let owner = owner.clone();
                    menu = menu.item(
                        PopupMenuItem::new(choice.label.clone())
                            .checked(
                                chosen
                                    .as_ref()
                                    .is_some_and(|(p, m)| p == &provider && m == &model),
                            )
                            .on_click(move |_, _, cx| {
                                let _ = owner.update(cx, |this, cx| {
                                    if !this.is_busy(cx) && !this.is_streaming(cx) && !this.history_only {
                                        this.binding = Some((provider.clone(), model.clone()));
                                        cx.emit(ConversationEvent::BindingChanged);
                                        cx.notify();
                                    }
                                });
                            }),
                    );
                }
                let owner = owner.clone();
                menu.separator()
                    .item(PopupMenuItem::new("配置供应商…").on_click(move |_, _, cx| {
                        let _ = owner
                            .update(cx, |_, cx| cx.emit(ConversationEvent::ConfigureProviders));
                    }))
            })
            .into_any_element()
    }

    fn composer(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let streaming = self.is_streaming(cx) || self.attachment_task.is_some();
        let focused = self.input.read(cx).focus_handle(cx).is_focused(window);
        div()
            .v_flex()
            .w_full()
            .gap_2()
            .when(!self.attachments.is_empty(), |column| {
                column
                    .child(
                        div()
                            .id("attachment-strip")
                            .flex()
                            .gap_2()
                            .overflow_x_scroll()
                            .children(self.attachments.iter().map(|path| {
                                let remove = path.clone();
                                let name = path
                                    .file_name()
                                    .unwrap_or_default()
                                    .to_string_lossy()
                                    .into_owned();
                                div()
                                    .flex()
                                    .items_center()
                                    .flex_shrink_0()
                                    .gap_2()
                                    .p_2()
                                    .rounded_lg()
                                    .border_1()
                                    .border_color(cx.theme().border)
                                    .bg(cx.theme().muted)
                                    .child(if let Some(image) = self.thumbnails.get(path) {
                                        img(image.clone())
                                            .size(rems(2.5))
                                            .object_fit(ObjectFit::Contain)
                                            .into_any_element()
                                    } else {
                                        Icon::new(IconName::File).small().into_any_element()
                                    })
                                    .child(
                                        div()
                                            .v_flex()
                                            .max_w(rems(14.))
                                            .child(
                                                div()
                                                    .text_sm()
                                                    .overflow_hidden()
                                                    .text_ellipsis()
                                                    .child(name.clone()),
                                            )
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .text_color(cx.theme().muted_foreground)
                                                    .child(if self.thumbnails.contains_key(path) {
                                                        "图片"
                                                    } else {
                                                        "附件"
                                                    }),
                                            ),
                                    )
                                    .child(
                                        Button::new(SharedString::from(format!(
                                            "attachment-{}",
                                            path.display()
                                        )))
                                        .small()
                                        .ghost()
                                        .icon(IconName::X)
                                        .disabled(self.is_busy(cx))
                                        .tooltip("移除附件")
                                        .accessibility_label(format!("移除附件 {name}"))
                                        .on_click(
                                            cx.listener(move |this, _, _, cx| {
                                                this.remove_attachment(&remove, cx);
                                            }),
                                        ),
                                    )
                            })),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("图片随请求发送；文档和文件夹由 Agent 工具读取。"),
                    )
            })
            .when_some(self.attachment_error.clone(), |column, error| {
                column.child(div().text_xs().text_color(cx.theme().danger).child(error))
            })
            .child(
                div()
                    .id("composer-surface")
                    .drag_over::<ExternalPaths>(|style, _, _, cx| {
                        style.bg(cx.theme().muted).border_color(cx.theme().primary)
                    })
                    .on_drop(cx.listener(|this, paths: &ExternalPaths, window, cx| {
                        this.add_attachments(paths.paths(), cx);
                        this.input.update(cx, |input, cx| input.focus(window, cx));
                    }))
                    // Port of Zeron's expanded pill: text above, attachments
                    // on the left and model/send controls on the right.
                    .v_flex()
                    .w_full()
                    .min_h(rems(8.))
                    .p_3()
                    .gap_3()
                    .rounded(rems(1.25))
                    .border_1()
                    .border_color(if focused {
                        cx.theme().ring
                    } else {
                        cx.theme().border
                    })
                    .bg(cx.theme().group_box)
                    .child(
                        div().w_full().min_w_0().flex_1().px_1().py_2().child(
                            Textarea::new(&self.input)
                                .appearance(false)
                                .bordered(false)
                                .aria_label("消息"),
                        ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                Button::new("attachments")
                                    .small()
                                    .ghost()
                                    .icon(IconName::Paperclip)
                                    .accessibility_label("添加附件")
                                    .tooltip("添加附件，也可拖入输入框")
                                    .disabled(
                                        self.picker.is_some()
                                            || self.is_busy(cx)
                                            || self.history_only,
                                    )
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.choose_attachments(cx)),
                                    ),
                            )
                            .child(div().flex_1())
                            .child(
                                div()
                                    .min_w_0()
                                    .max_w(relative(0.6))
                                    .overflow_hidden()
                                    .child(self.model_picker(cx)),
                            )
                            .child(if streaming {
                                Button::new("stop")
                                    .small()
                                    .icon(IconName::Square)
                                    .accessibility_label("停止生成")
                                    .tooltip("停止生成 · Esc")
                                    .on_click(cx.listener(|this, _, _, cx| this.stop(cx)))
                            } else {
                                Button::new("send")
                                    .small()
                                    .primary()
                                    .icon(IconName::ArrowUp)
                                    .accessibility_label("发送消息")
                                    .tooltip("发送 · Enter")
                                    .disabled(
                                        self.is_busy(cx)
                                            || self.history_only
                                            || (self.input.read(cx).value().trim().is_empty()
                                                && self.attachments.is_empty()),
                                    )
                                    .on_click(
                                        cx.listener(|this, _, window, cx| this.send(window, cx)),
                                    )
                            }),
                    ),
            )
            .child(
                div()
                    .px_3()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(if streaming {
                        "Esc 停止 · 可继续编辑草稿"
                    } else {
                        "Shift + Enter 换行"
                    }),
            )
            .into_any_element()
    }
}

impl Render for Preview {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let empty = self.transcript.read(cx).count() == 0;
        div()
            .v_flex()
            .size_full()
            .min_w_0()
            .capture_key_down(cx.listener(Self::input_key))
            .capture_action(cx.listener(Self::paste_attachments))
            .capture_action(
                cx.listener(|this, _: &MoveUp, window, cx| this.browse_history(true, window, cx)),
            )
            .capture_action(
                cx.listener(|this, _: &MoveDown, window, cx| {
                    this.browse_history(false, window, cx)
                }),
            )
            .on_action(|_: &gpui_kit::base::input::Enter, _, _| {})
            .child(if empty {
                div()
                    .v_flex()
                    .flex_1()
                    .min_h_0()
                    .justify_center()
                    .items_center()
                    .px_6()
                    .pb_16()
                    .child(
                        div()
                            .v_flex()
                            .w_full()
                            .max_w(rems(50.))
                            .child(
                                div()
                                    .flex()
                                    .justify_center()
                                    .mb_6()
                                    .child(self.momo.clone()),
                            )
                            .child(self.composer(window, cx)),
                    )
                    .into_any_element()
            } else {
                div()
                    .v_flex()
                    .flex_1()
                    .min_h_0()
                    .when_some(self.fork_title.clone(), |column, title| {
                        column.child(
                            div()
                                .px_6()
                                .child(crate::transcript::fork_marker(title, cx)),
                        )
                    })
                    .child(div().flex_1().min_h_0().child(self.transcript.clone()))
                    .child(
                        div().px_6().pt_3().pb_4().flex().justify_center().child(
                            div()
                                .w_full()
                                .max_w(rems(50.))
                                .child(self.composer(window, cx)),
                        ),
                    )
                    .into_any_element()
            })
    }
}
#[cfg(test)]
mod tests {
    use super::Preview;
    use gpui_kit::{AppContext, Bounds, TestAppContext, WindowBounds, WindowOptions, px, size};

    // The application's actual input subscription must submit once, preserve a
    // draft while busy, and treat Shift+Enter as editing rather than sending.
    #[gpui_kit::test]
    fn keyboard_submission_preserves_the_next_draft(cx: &mut TestAppContext) {
        let (window, view) = cx.update(|cx| {
            gpui_kit::init(cx);
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                        None,
                        size(px(900.), px(680.)),
                        cx,
                    ))),
                    ..Default::default()
                },
                cx,
                |window, cx| {
                    cx.new(|cx| {
                        let mut view = Preview::new(window, cx);
                        view.load_sample(0, cx);
                        view
                    })
                },
            )
            .unwrap()
        });
        cx.simulate_input(window, "你好 🌿");
        cx.simulate_keystrokes(window, "shift-enter");
        cx.update(|cx| {
            let view = view.read(cx);
            assert_eq!(view.transcript.read(cx).count(), 0);
            assert_eq!(view.input.read(cx).value(), "你好 🌿\n");
        });
        cx.simulate_input(window, "第二行");
        cx.simulate_keystrokes(window, "enter");
        cx.update(|cx| {
            let view = view.read(cx);
            assert_eq!(view.transcript.read(cx).count(), 2);
            assert!(view.transcript.read(cx).is_streaming());
            assert_eq!(view.input.read(cx).value(), "");
        });
        cx.simulate_input(window, "下一条草稿");
        cx.simulate_keystrokes(window, "enter");
        cx.update(|cx| {
            let view = view.read(cx);
            assert_eq!(view.transcript.read(cx).count(), 2);
            assert_eq!(view.input.read(cx).value(), "下一条草稿");
        });
    }
    // Original composer contract: history browsing preserves the unfinished
    // draft; Esc only cancels generation when the editor owns focus.
    #[gpui_kit::test]
    fn history_and_rewind_preserve_text_until_explicit_confirmation(cx: &mut TestAppContext) {
        let (window, view) = cx.update(|cx| {
            gpui_kit::init(cx);
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| {
                    let mut view = Preview::new(window, cx);
                    view.load_sample(0, cx);
                    view
                })
            })
            .unwrap()
        });
        cx.simulate_input(window, "第一条提问\n包含第二行 🌿");
        cx.simulate_keystrokes(window, "enter");
        cx.run_until_parked();
        cx.simulate_keystrokes(window, "escape");
        view.read_with(cx, |view, cx| assert!(!view.is_streaming(cx)));
        cx.simulate_input(window, "未发送的草稿");
        cx.simulate_keystrokes(window, "up");
        view.read_with(cx, |view, cx| {
            assert_eq!(view.input.read(cx).value(), "第一条提问\n包含第二行 🌿")
        });
        cx.simulate_keystrokes(window, "down");
        view.read_with(cx, |view, cx| {
            assert_eq!(view.input.read(cx).value(), "未发送的草稿")
        });
        window
            .update(cx, |_, window, cx| {
                view.update(cx, |view, cx| view.confirm_rewind(0, window, cx))
            })
            .unwrap();
        cx.simulate_keystrokes(window, "escape");
        view.read_with(cx, |view, cx| {
            assert_eq!(view.transcript.read(cx).count(), 2);
            assert_eq!(view.input.read(cx).value(), "未发送的草稿");
        });
        window
            .update(cx, |_, window, cx| {
                view.update(cx, |view, cx| view.confirm_rewind(0, window, cx))
            })
            .unwrap();
        cx.simulate_keystrokes(window, "enter");
        view.read_with(cx, |view, cx| {
            assert_eq!(view.transcript.read(cx).count(), 0);
            assert_eq!(view.input.read(cx).value(), "第一条提问\n包含第二行 🌿");
        });
        cx.simulate_keystrokes(window, "enter");
        view.read_with(cx, |view, cx| {
            assert_eq!(view.transcript.read(cx).count(), 2)
        });
    }
    #[gpui_kit::test]
    fn missing_provider_keeps_draft_without_fabricating_a_reply(cx: &mut TestAppContext) {
        let (window, view) = cx.update(|cx| {
            gpui_kit::init(cx);
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| Preview::new(window, cx))
            })
            .unwrap()
        });
        cx.simulate_input(window, "保留我的提问 🌿");
        cx.simulate_keystrokes(window, "enter");
        cx.run_until_parked();
        view.read_with(cx, |view, cx| {
            assert_eq!(view.input.read(cx).value(), "保留我的提问 🌿");
            assert_eq!(view.transcript.read(cx).count(), 0);
            assert!(!view.is_streaming(cx));
        });
    }
}

#[cfg(test)]
#[path = "preview/attachment_tests.rs"]
mod attachment_tests;
