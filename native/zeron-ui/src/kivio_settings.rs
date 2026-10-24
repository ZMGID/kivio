//! Kivio's settings information architecture in the imported Zeron shell.
//! Unsupported host features remain visibly read-only, never successful no-op controls.
mod controls;
mod hotkeys;
mod mixer;
use crate::{settings::widgets, shell::SettingsSection, theme::Theme, typography};
use gpui_kit::component::{
    Disableable, Selectable, Sizable, StyledExt,
    button::Button,
    input::{Input, InputEvent, InputState, Textarea, TextareaState},
    menu::{DropdownMenu, PopupMenuItem},
    switch::Switch,
};
use gpui_kit::{prelude::*, *};
use kivio_model_runtime::{
    native_preferences::{self, Change, TextPreference},
    settings_config::Settings,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Translate,
    Lens,
    Chat,
    Memory,
    Mixer,
    Cli,
    Computer,
    Hooks,
    Search,
    Usage,
    Backup,
    About,
}
impl Section {
    const ALL: [Self; 12] = [
        Self::Translate,
        Self::Lens,
        Self::Chat,
        Self::Memory,
        Self::Mixer,
        Self::Cli,
        Self::Computer,
        Self::Hooks,
        Self::Search,
        Self::Usage,
        Self::Backup,
        Self::About,
    ];
    pub fn slug(self) -> &'static str {
        match self {
            Self::Translate => "translate",
            Self::Lens => "lens",
            Self::Chat => "chat",
            Self::Memory => "memory",
            Self::Mixer => "mixer",
            Self::Cli => "externalAgents",
            Self::Computer => "computerControl",
            Self::Hooks => "hooks",
            Self::Search => "webSearch",
            Self::Usage => "usage",
            Self::Backup => "backup",
            Self::About => "about",
        }
    }
    pub fn from_slug(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|section| section.slug() == value)
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Translate => "翻译",
            Self::Lens => "Lens",
            Self::Chat => "Kivio Agent",
            Self::Memory => "记忆",
            Self::Mixer => "混音器",
            Self::Cli => "本地 CLI 管理",
            Self::Computer => "电脑操控",
            Self::Hooks => "Hooks",
            Self::Search => "网络搜索",
            Self::Usage => "使用统计",
            Self::Backup => "备份与同步",
            Self::About => "关于",
        }
    }
}

pub fn page_frame(title: &str, subtitle: &str, content: AnyElement, cx: &App) -> AnyElement {
    let theme = Theme::of(cx).for_settings_surface();
    widgets::scroll_faded(
        format!("kivio-settings-scroll-{title}"),
        div()
            .id(SharedString::from(format!(
                "kivio-settings-content-{title}"
            )))
            .size_full()
            .overflow_y_scroll()
            .child(
                widgets::page_column()
                    .text_size(typography::ui_rems(13.))
                    .text_color(theme.text)
                    .child(widgets::page_header(&theme, title, None))
                    .child(widgets::page_subtitle(&theme, subtitle.to_owned()))
                    .child(div().mt_8().child(content)),
            ),
    )
    .into_any_element()
}

pub struct SettingsPage {
    pub section: SettingsSection,
    settings: Option<Settings>,
    settings_value: serde_json::Value,
    observed_revision: u64,
    snapshot_refresh: Option<Task<()>>,
    directory: std::path::PathBuf,
    busy: bool,
    focus: FocusHandle,
    recording: Option<&'static str>,
    recording_listener: Option<Subscription>,
    recording_blur: Option<Subscription>,
    expanded_prompts: std::collections::HashSet<TextPreference>,
    prompts: std::collections::HashMap<TextPreference, Entity<TextareaState>>,
    name: Option<Entity<InputState>>,
    fields: std::collections::HashMap<String, Entity<InputState>>,
    editor_baselines: std::collections::HashMap<String, String>,
    text_baselines: std::collections::HashMap<TextPreference, serde_json::Value>,
    field_baselines: std::collections::HashMap<String, serde_json::Value>,
    _editors: Vec<Subscription>,
    providers: Entity<crate::providers::ProviderSettings>,
    computer: Entity<crate::settings::computer::ComputerTools>,
    model_database: Entity<crate::settings::model_database::ModelDatabaseUpdate>,
    offline: Entity<crate::settings::offline::OfflinePackages>,
    custom_themes: Entity<crate::settings::custom_themes::CustomThemes>,
    error: Option<String>,
    feedback: Option<(String, Result<(), String>)>,
    _load: Task<()>,
    directory_prompt: Option<Task<()>>,
    #[cfg(target_os = "macos")]
    permissions: (bool, bool),
    #[cfg(target_os = "macos")]
    _permission_activation: Subscription,
}
impl SettingsPage {
    fn refresh_confirmed_snapshot(&mut self, window: &Window, cx: &mut Context<Self>) {
        let revision = native_preferences::revision();
        if self.settings.is_none() || self.busy || self.snapshot_refresh.is_some() || self.observed_revision == revision { return; }
        let directory = self.directory.clone();
        let handle = window.window_handle();
        self.snapshot_refresh = Some(cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(async move { native_preferences::load(&directory) }).await;
            let _ = cx.update_window(handle, |_, window, cx| this.update(cx, |this, cx| {
                this.snapshot_refresh = None;
                if !this.busy && native_preferences::revision() == revision {
                    this.observed_revision = revision;
                    match result {
                        Ok(settings) => {
                            let document = serde_json::to_value(&settings).expect("serializable settings");
                            let values = this.editor_values(cx);
                            for (path, input) in &this.fields {
                                if values.get(path) == this.editor_baselines.get(path) {
                                    let value = document.pointer(path).cloned().unwrap_or_default();
                                    let text = value.as_str().map(str::to_owned).unwrap_or_else(|| value.to_string());
                                    if values.get(path) != Some(&text) { input.update(cx, |input, cx| input.set_value(text.clone(), window, cx)); }
                                    this.editor_baselines.insert(path.clone(), text);
                                    this.field_baselines.insert(path.clone(), value);
                                }
                            }
                            for (field, input) in &this.prompts {
                                let key = format!("{field:?}");
                                if values.get(&key) == this.editor_baselines.get(&key) {
                                    let value = field.value(&settings);
                                    let text = value.as_str().filter(|value| !value.is_empty()).map(str::to_owned)
                                        .unwrap_or_else(|| default_prompt(*field, &settings));
                                    if values.get(&key) != Some(&text) { input.update(cx, |input, cx| input.set_value(text.clone(), window, cx)); }
                                    this.editor_baselines.insert(key, text);
                                    this.text_baselines.insert(*field, value);
                                }
                            }
                            if values.get("UserName") == this.editor_baselines.get("UserName") {
                                if let Some(input) = &this.name {
                                    let text = settings.chat.user_display_name.clone();
                                    if values.get("UserName") != Some(&text) { input.update(cx, |input, cx| input.set_value(text.clone(), window, cx)); }
                                    this.editor_baselines.insert("UserName".into(), text);
                                    this.text_baselines.insert(TextPreference::UserName, TextPreference::UserName.value(&settings));
                                }
                            }
                            this.settings_value = document;
                            this.settings = Some(settings);
                        }
                        Err(error) => this.error = Some(error),
                    }
                }
                cx.notify();
            }));
        }));
    }
    pub fn new(
        directory: std::path::PathBuf,
        providers: Entity<crate::providers::ProviderSettings>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let load_directory = directory.clone();
        let handle = window.window_handle();
        let task = cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move { native_preferences::load(&load_directory) })
                .await;
            let _ = cx.update_window(handle, |_, window, cx| {
                this.update(cx, |this, cx| {
                    match result {
                        Ok(settings) => {
                            let fields = [
                                (
                                    TextPreference::InputPrompt,
                                    settings.translator_prompt.clone().unwrap_or_default(),
                                ),
                                (
                                    TextPreference::ScreenshotPrompt,
                                    settings
                                        .screenshot_translation
                                        .prompt
                                        .clone()
                                        .unwrap_or_default(),
                                ),
                                (
                                    TextPreference::TextPrompt,
                                    settings
                                        .screenshot_translation
                                        .text_prompt
                                        .clone()
                                        .unwrap_or_default(),
                                ),
                                (
                                    TextPreference::ReplacementPrompt,
                                    settings
                                        .screenshot_translation
                                        .replace_prompt
                                        .clone()
                                        .unwrap_or_default(),
                                ),
                                (
                                    TextPreference::LensSystemPrompt,
                                    settings.lens.system_prompt.clone(),
                                ),
                                (
                                    TextPreference::LensQuestionPrompt,
                                    settings.lens.question_prompt.clone(),
                                ),
                                (
                                    TextPreference::AgentPrompt,
                                    settings.chat.system_prompt.clone(),
                                ),
                                (
                                    TextPreference::ChatPrompt,
                                    settings.chat.chat_mode.system_prompt.clone(),
                                ),
                                (
                                    TextPreference::PromptOptimize,
                                    settings.chat.prompt_optimize_prompt.clone(),
                                ),
                            ];
                            this.prompts = fields
                                .into_iter()
                                .map(|(field, text)| {
                                    let text = if text.is_empty() {
                                        default_prompt(field, &settings)
                                    } else {
                                        text
                                    };
                                    (
                                        field,
                                        cx.new(|cx| {
                                            TextareaState::new(window, cx)
                                                .rows(5)
                                                .default_value(text)
                                        }),
                                    )
                                })
                                .collect();
                            this.name = Some(cx.new(|cx| {
                                InputState::new(window, cx)
                                    .default_value(settings.chat.user_display_name.clone())
                            }));
                            let document = serde_json::to_value(&settings).unwrap_or_default();
                            this.field_baselines = controls::TEXT_FIELDS.iter().map(|path| (path.to_string(), document.pointer(path).cloned().unwrap_or_default())).collect();
                            this.fields = controls::TEXT_FIELDS
                                .iter()
                                .map(|path| {
                                    let value = document.pointer(path).cloned().unwrap_or_default();
                                    let text = value
                                        .as_str()
                                        .map(str::to_string)
                                        .unwrap_or_else(|| value.to_string());
                                    (
                                        path.to_string(),
                                        cx.new(|cx| {
                                            InputState::new(window, cx)
                                                .masked(path.ends_with("Token"))
                                                .default_value(text)
                                        }),
                                    )
                                })
                                .collect();
                            this.text_baselines = this.prompts.keys().copied().chain(std::iter::once(TextPreference::UserName)).map(|field| (field, field.value(&settings))).collect();
                            this.track_editors(cx);
                            this.settings_value = document;
                            this.settings = Some(settings);
                        }
                        Err(error) => this.error = Some(error),
                    };
                    cx.notify();
                })
            });
        });
        let custom_themes = cx.new(|cx| {
            crate::settings::custom_themes::CustomThemes::new(directory.clone(), window, cx)
        });
        let model_database = cx.new(|cx| crate::settings::model_database::ModelDatabaseUpdate::new(directory.clone(), cx));
        let computer = cx.new(|cx| crate::settings::computer::ComputerTools::new(directory.clone(), cx));
        #[cfg(target_os = "macos")]
        let permission_activation = cx.observe_window_activation(window, |this, window, cx| {
            if window.is_window_active() {
                this.permissions = permission_status();
                cx.notify();
            }
        });
        let offline = crate::settings::offline::OfflinePackages::retained(directory.clone(), cx);
        Self {
            offline,
            #[cfg(target_os = "macos")]
            permissions: permission_status(),
            #[cfg(target_os = "macos")]
            _permission_activation: permission_activation,
            computer,
            model_database,
            custom_themes,
            section: SettingsSection::General,
            settings: None,
            settings_value: serde_json::Value::Null,
            observed_revision: native_preferences::revision(),
            snapshot_refresh: None,
            directory,
            busy: false,
            focus: cx.focus_handle(),
            recording: None,
            recording_listener: None,
            recording_blur: None,
            prompts: Default::default(),
            expanded_prompts: Default::default(),
            fields: Default::default(),
            editor_baselines: Default::default(),
            text_baselines: Default::default(),
            field_baselines: Default::default(),
            _editors: vec![],
            providers,
            name: None,
            error: None,
            feedback: None,
            _load: task,
            directory_prompt: None,
        }
    }
    pub(crate) fn is_busy(&self, cx: &App) -> bool {
        self.busy || self.model_database.read(cx).is_busy() || self.custom_themes.read(cx).is_busy(cx) || self.computer.read(cx).is_busy(cx) || self.offline.read(cx).is_busy()
    }
    pub(crate) fn has_unsaved_changes(&self, cx: &App) -> bool {
        self.custom_themes.read(cx).has_unsaved_changes(cx)
            || self.editor_values(cx).iter().any(|(key, value)| {
                self.editor_baselines.get(key).is_some_and(|baseline| baseline != value)
            })
    }
    fn editor_values(&self, cx: &App) -> std::collections::HashMap<String, String> {
        let mut values: std::collections::HashMap<_, _> = self.fields.iter()
            .map(|(path, input)| (path.clone(), input.read(cx).value().to_string())).collect();
        values.extend(self.prompts.iter().map(|(field, input)| (format!("{field:?}"), input.read(cx).value().to_string())));
        if let Some(name) = &self.name {
            values.insert("UserName".into(), name.read(cx).value().to_string());
        }
        values
    }
    fn track_editors(&mut self, cx: &mut Context<Self>) {
        // Snapshot what was displayed, including generated default prompts. A later
        // language change must not turn an untouched default into an unsaved edit.
        self.editor_baselines = self.editor_values(cx);
        let mut subscriptions = Vec::new();
        for (path, input) in &self.fields {
            let key = path.clone();
            subscriptions.push(cx.subscribe(input, move |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) { this.editor_changed(&key, cx); }
            }));
        }
        for (field, input) in &self.prompts {
            let key = format!("{field:?}");
            subscriptions.push(cx.subscribe(input, move |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) { this.editor_changed(&key, cx); }
            }));
        }
        if let Some(name) = &self.name {
            subscriptions.push(cx.subscribe(name, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) { this.editor_changed("UserName", cx); }
            }));
        }
        self._editors = subscriptions;
    }
    fn editor_changed(&mut self, key: &str, cx: &mut Context<Self>) {
        if self.feedback.as_ref().is_some_and(|(field, _)| field == key) {
            self.feedback = None;
        }
        cx.notify();
    }

    fn error_notice(&self, cx: &Context<Self>) -> Div {
        let Some(error) = &self.error else {
            return div();
        };
        div().flex().flex_col().gap_2().child(error.clone()).child(
            Button::new("reload-settings")
                .small()
                .label(crate::i18n::text("放弃未保存编辑并重新读取"))
                .disabled(self.busy)
                .on_click(cx.listener(|this, _, window, cx| {
                    let section = this.section;
                    *this = Self::new(this.directory.clone(), this.providers.clone(), window, cx);
                    this.section = section;
                    cx.notify();
                })),
        )
    }
    fn save_text(&mut self, field: TextPreference, after: String, cx: &mut Context<Self>) {
        let Some(settings) = &self.settings else { return; };
        let before = self.text_baselines.get(&field).cloned().unwrap_or_else(|| field.value(settings));
        self.save(Change::Text { field, before, after }, cx);
    }
    fn save(&mut self, change: Change, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.error = None;
        self.feedback = None;
        let field = match &change {
            Change::Fields(edits) if edits.len() == 1 => Some(edits[0].path.clone()),
            Change::Text { field, .. } => Some(format!("{field:?}")),
            _ => None,
        };
        let text_field = match &change { Change::Text { field, .. } => Some(*field), _ => None };
        let edited_fields: Vec<String> = match &change { Change::Fields(edits) => edits.iter().map(|edit| edit.path.clone()).collect(), _ => vec![] };
        let undo_hotkeys = match &change {
            Change::Fields(edits) if edits.iter().any(|edit| matches!(edit.path.as_str(), "/hotkey" | "/chatHotkey" | "/closeChatHotkey" | "/screenshotTranslation/textHotkey" | "/screenshotTranslation/hotkey" | "/screenshotTranslation/enabled" | "/lens/hotkey" | "/lens/enabled")) => Some(edits.iter().map(|edit| native_preferences::FieldEdit { path: edit.path.clone(), before: edit.after.clone(), after: edit.before.clone() }).collect::<Vec<_>>()),
            _ => None,
        };
        let directory = self.directory.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    if let Change::Fields(edits) = &change {
                        if edits.len() == 1 && edits[0].path == "/launchAtStartup" {
                            return kivio_model_runtime::startup::save(
                                &directory,
                                edits[0].clone(),
                            );
                        }
                    }
                    native_preferences::save(&directory, change)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if let (Ok(settings), Some(undo)) = (&result, undo_hotkeys) {
                    if let Err(error) = crate::global_hotkeys::apply(settings, cx) {
                        let directory = this.directory.clone();
                        cx.spawn(async move |this, cx| {
                            let restored = cx.background_spawn(async move { native_preferences::save(&directory, Change::Fields(undo)) }).await;
                            let _ = this.update(cx, |this, cx| {
                                this.busy = false;
                                this.error = Some(match restored { Ok(settings) => { this.settings_value = serde_json::to_value(&settings).expect("serializable settings"); this.settings = Some(settings); error }, Err(rollback) => crate::ui_format!("{error}；配置回滚失败：{rollback}。请重新读取设置。", "{error}; configuration rollback failed: {rollback}. Reload settings.") });
                                cx.notify();
                            });
                        }).detach();
                        return;
                    }
                }
                this.busy = false;
                this.feedback = field
                    .clone()
                    .map(|field| (field, result.as_ref().map(|_| ()).map_err(Clone::clone)));
                match result {
                    Ok(settings) => {
                        if let Some(field) = text_field {
                            this.text_baselines.insert(field, field.value(&settings));
                        }
                        let confirmed = serde_json::to_value(&settings).expect("serializable settings");
                        let editor_values = this.editor_values(cx);
                        for path in &edited_fields {
                            if let Some(baseline) = this.field_baselines.get_mut(path) {
                                if let Some(value) = confirmed.pointer(path) { *baseline = value.clone(); }
                                if let Some(value) = editor_values.get(path) { this.editor_baselines.insert(path.clone(), value.clone()); }
                            }
                        }
                        if let Some(key) = &field {
                            if let Some(value) = this.editor_values(cx).remove(key) {
                                this.editor_baselines.insert(key.clone(), value);
                            }
                        }
                        if field.as_deref() == Some("/theme") {
                            crate::appearance::set_mode(
                                match settings.theme.as_str() {
                                    "light" => crate::appearance::AppearanceMode::Light,
                                    "dark" => crate::appearance::AppearanceMode::Dark,
                                    _ => crate::appearance::AppearanceMode::System,
                                },
                                cx,
                            );
                        }
                        if edited_fields.iter().any(|field| field == "/settingsLanguage") {
                            crate::i18n::apply(settings.settings_language.as_deref(), cx);
                        }
                        this.settings_value = confirmed;
                        this.settings = Some(settings);
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}

pub(crate) fn row(label: impl Into<SharedString>, control: AnyElement, theme: &Theme) -> Div {
    div()
        .flex()
        .flex_wrap()
        .items_center()
        .justify_between()
        .gap_4()
        .min_h(px(60.))
        .py_3()
        .child(
            div()
                .flex_1()
                .min_w(rems(8.))
                .text_size(typography::ui_rems(13.))
                .font_weight(FontWeight::MEDIUM)
                .child(crate::i18n::display(label)),
        )
        .child(
            div()
                .flex_none()
                .max_w_full()
                .text_color(theme.text_muted)
                .child(control),
        )
}
fn value(label: &'static str, value: impl Into<SharedString>, theme: &Theme) -> Div {
    row(
        label,
        div()
            .text_color(theme.text_muted)
            .child(value.into())
            .into_any_element(),
        theme,
    )
}
fn pending(label: &'static str, checked: bool, theme: &Theme) -> Div {
    row(
        label,
        Switch::new(label)
            .accessibility_label(crate::i18n::text(label))
            .checked(checked)
            .disabled(true)
            .into_any_element(),
        theme,
    )
}
pub(crate) fn card(title: &'static str, rows: Vec<Div>, theme: &Theme) -> Div {
    widgets::section(
        theme,
        title,
        widgets::section_card(theme)
            .mt_0()
            .px_4()
            .children(rows.into_iter().enumerate().map(|(index, row)| {
                row.when(index > 0, |row| {
                    row.border_t_1().border_color(widgets::row_divider(theme))
                })
            })),
    )
    .mt_0()
}
fn unavailable(theme: &Theme) -> Div {
    div()
        .text_color(theme.text_muted)
        .text_size(px(13.))
        .child(crate::i18n::text("部分功能尚未迁移，灰色选项和只读信息暂不可修改。"))
}
fn font_picker(code: bool, cx: &App) -> AnyElement {
    let current = if code {
        typography::code_requested(cx)
    } else {
        typography::requested(cx)
    };
    let choices = typography::availability(cx).choices().to_vec();
    Button::new(if code { "code-font" } else { "ui-font" })
        .small()
        .label(current.label().to_owned())
        .dropdown_caret(true)
        .dropdown_menu(move |mut menu, _, _| {
            for family in &choices {
                let family = family.clone();
                menu = menu.item(
                    PopupMenuItem::new(family.label().to_owned())
                        .checked(family == current)
                        .on_click(move |_, _, cx| {
                            if code {
                                typography::set_code_family(family.clone(), cx);
                            } else {
                                typography::set_family(family.clone(), cx);
                            }
                        }),
                );
            }
            menu
        })
        .into_any_element()
}
impl Render for SettingsPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.refresh_confirmed_snapshot(_window, cx);
        let theme = Theme::of(cx).for_settings_surface();
        let Some(s) = &self.settings else {
            return page_frame(
                self.section.label(),
                "",
                div()
                    .child(self.error.clone().unwrap_or("正在读取 Kivio 设置…".into()))
                    .child(self.error_notice(cx))
                    .into_any_element(),
                cx,
            );
        };
        let mut body = div().flex().flex_col().gap_8();
        body = body.when(self.error.is_some(), |body| {
            body.child(self.error_notice(cx))
        });

        if let SettingsSection::Kivio(
            section @ (Section::Translate | Section::Lens | Section::Chat),
        ) = self.section
        {
            return self.render_detail(section, &theme, cx);
        }
        if self.section == SettingsSection::Kivio(Section::Mixer) {
            return page_frame(
                "混音器",
                "为辅助任务和子代理分配模型。",
                div()
                    .child(self.error_notice(cx))
                    .child(self.mixer(&theme, cx))
                    .into_any_element(),
                cx,
            );
        }
        let subtitle;
        match self.section {
            SettingsSection::General => {
                subtitle = "外观、行为、归档和权限。";
                let size = typography::font_size(cx);
                let size_picker = Button::new("ui-size")
                    .small()
                    .label(size.label())
                    .dropdown_caret(true)
                    .dropdown_menu(move |mut menu, _, _| {
                        for choice in typography::UiFontSize::ALL {
                            menu = menu.item(
                                PopupMenuItem::new(choice.label())
                                    .checked(choice == size)
                                    .on_click(move |_, window, cx| {
                                        typography::set_font_size(choice, window, cx);
                                    }),
                            );
                        }
                        menu
                    });
                body = body
                    .child(card(
                        "外观",
                        vec![
                            self.choice("界面语言", "/settingsLanguage", &[("zh", "中文"), ("en", "English")], &theme, cx),
                            row("界面字号", size_picker.into_any_element(), &theme),
                            row("界面字体", font_picker(false, cx), &theme),
                            row("代码字体", font_picker(true, cx), &theme),
                        ],
                        &theme,
                    ))
                    .child(unavailable(&theme))
                    .child(card(
                        "行为",
                        vec![
                            self.toggle("开机启动", "/launchAtStartup", false, &theme, cx),
                            self.toggle("启动后最小化到托盘", "/launchMinimizedToTray", false, &theme, cx),
                            self.toggle("关闭时保持聊天窗口", "/keepChatWindowAlive", false, &theme, cx),
                            self.toggle("回复完成通知", "/chatCompletionNotifications", false, &theme, cx),
                            row(
                                "自动重试",
                                Switch::new("retry-enabled")
                                    .accessibility_label(crate::i18n::text("自动重试"))
                                    .checked(s.retry_enabled)
                                    .disabled(self.busy)
                                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                        this.save(Change::Retry(*checked), cx)
                                    }))
                                    .into_any_element(),
                                &theme,
                            ),
                            row(
                                "重试次数",
                                Button::new("retry-count").small()
                                    .label(s.retry_attempts.saturating_sub(1).to_string())
                                    .disabled(self.busy || !s.retry_enabled)
                                    .dropdown_caret(true)
                                    .dropdown_menu({
                                        let owner = cx.weak_entity();
                                        move |mut menu, _, _| {
                                            for count in 0..=10u8 {
                                                let owner = owner.clone();
                                                menu = menu.item(
                                                    PopupMenuItem::new(count.to_string()).on_click(
                                                        move |_, _, cx| {
                                                            let _ = owner.update(cx, |this, cx| {
                                                                this.save(
                                                                    Change::RetryAttempts(
                                                                        count + 1,
                                                                    ),
                                                                    cx,
                                                                )
                                                            });
                                                        },
                                                    ),
                                                );
                                            }
                                            menu
                                        }
                                    })
                                    .into_any_element(),
                                &theme,
                            ),
                        ],
                        &theme,
                    ))
                    .when(cfg!(target_os = "macos"), |body| {
                        #[cfg(target_os = "macos")]
                        { body.child(card("系统权限", vec![
                            row("辅助功能", Button::new("accessibility-settings").small().label(if self.permissions.0 { "已授权 · 管理…" } else { "未授权 · 前往设置…" }).on_click(|_, _, cx| cx.open_url("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")).into_any_element(), &theme),
                            row("屏幕录制", Button::new("screen-recording-settings").small().label(if self.permissions.1 { "已授权 · 管理…" } else { "未授权 · 前往设置…" }).on_click(|_, _, cx| cx.open_url("x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture")).into_any_element(), &theme),
                        ], &theme)) }
                        #[cfg(not(target_os = "macos"))]
                        { body }
                    })
                    .child(card(
                        "首次使用",
                        vec![row("设置引导", Button::new("restart-native-setup").small()
                            .label(crate::i18n::text("重新打开设置引导"))
                            .disabled(self.busy || self.has_unsaved_changes(cx))
                            .on_click(|_, window, cx| {
                                let id = window.window_handle().window_id();
                                if let Some((_, shell)) = crate::shell_windows(cx).into_iter().find(|(handle, _)| handle.window_id() == id) {
                                    cx.defer(move |cx| shell.update(cx, |shell, cx| shell.start_native_setup(cx)));
                                }
                            }).into_any_element(), &theme)],
                        &theme,
                    ));
            }
            SettingsSection::Appearance => {
                subtitle = "选择明暗模式与主题色板。";
                let current = crate::appearance::mode(cx);
                let modes = [
                    (crate::appearance::AppearanceMode::System, "跟随系统"),
                    (crate::appearance::AppearanceMode::Light, "浅色"),
                    (crate::appearance::AppearanceMode::Dark, "深色"),
                ];
                let label = modes.iter().find(|(mode, _)| *mode == current).unwrap().1;
                let owner = cx.entity().downgrade();
                let surface = crate::appearance::surface(cx);
                let surfaces = [
                    (zeron_theme::SurfacePreference::ThemeDefault, "跟随主题"),
                    (zeron_theme::SurfacePreference::Frosted, "毛玻璃"),
                    (zeron_theme::SurfacePreference::Opaque, "不透明"),
                ];
                let surface_label = surfaces.iter().find(|(value, _)| *value == surface).unwrap().1;
                body = body
                    .child(card(
                        "外观",
                        vec![row(
                            "明暗模式",
                            Button::new("theme-mode")
                                .small()
                                .label(crate::i18n::display(label))
                                .dropdown_caret(true)
                                .disabled(self.busy)
                                .dropdown_menu(move |mut menu, _, _| {
                                    for (mode, label) in modes {
                                        let owner = owner.clone();
                                        menu = menu.item(
                                            PopupMenuItem::new(label)
                                                .checked(mode == current)
                                                .on_click(move |_, _, cx| {
                                                    let _ = owner.update(cx, |this, cx| {
                                                        let Some(settings) = &this.settings else { return; };
                                                        let after = match mode { crate::appearance::AppearanceMode::Light => "light", crate::appearance::AppearanceMode::Dark => "dark", _ => "system" };
                                                        this.save(Change::Fields(vec![native_preferences::FieldEdit { path: "/theme".into(), before: serde_json::json!(settings.theme), after: serde_json::json!(after) }]), cx);
                                                    });
                                                }),
                                        );
                                    }
                                    menu
                                })
                                .into_any_element(),
                            &theme,
                        ), row(
                            "窗口材质",
                            Button::new("theme-surface")
                                .small()
                                .label(surface_label)
                                .dropdown_caret(true)
                                .dropdown_menu(move |mut menu, _, _| {
                                    for (value, label) in surfaces {
                                        menu = menu.item(PopupMenuItem::new(label)
                                            .checked(value == surface)
                                            .on_click(move |_, _, cx| crate::appearance::set_surface(value, cx)));
                                    }
                                    menu
                                })
                                .into_any_element(),
                            &theme,
                        )],
                        &theme,
                    ))
                    .child(theme_library_grid(self.custom_themes.clone(), cx)).child(self.custom_themes.clone());
            }
            SettingsSection::Shortcuts => {
                subtitle = "集中管理所有全局快捷键。";
                body = body.child(self.shortcuts(&theme, cx));
            }

            SettingsSection::Kivio(section) => {
                subtitle = match section {
                    Section::Translate => "输入翻译与快速翻译的语言、模型、OCR 和提示词。",
                    Section::Lens => "视觉问答的快捷键、响应方式和提示词。",
                    Section::Chat => "主对话模型、流式/思考、系统提示词。",
                    Section::Memory => "L1 注入；L2 按需读取。",
                    Section::Cli => "检测外部 CLI 编码代理，管理版本、路径、模型与环境变量。",
                    Section::Computer => "管理桌面和浏览器操作工具。",
                    Section::Hooks => "管理工作流事件触发的脚本。",
                    Section::Search => "配置搜索引擎和网页读取。",
                    Section::Usage => "查看模型调用和用量。",
                    Section::Backup => "管理本地备份、WebDAV 手动备份与恢复。",
                    Section::About => "Kivio 原生桌面客户端。",
                    Section::Mixer => "按副任务路由模型。",
                };
                if !matches!(section, Section::About | Section::Computer) {
                    body = body.child(unavailable(&theme));
                }
                let rows = match section {
                    Section::Translate | Section::Lens | Section::Chat => unreachable!(),
                    Section::Memory => vec![
                        pending("启用记忆", s.chat_memory.enabled, &theme),
                        value("L1 核心记忆", "chat-memory/L1.md", &theme),
                        value("L2 扩展记忆", "chat-memory/L2.md", &theme),
                    ],
                    Section::Cli => vec![
                        value("CLI 检测、版本与路径", "原生管理尚未接入", &theme),
                        value(
                            "模型、环境变量与账号",
                            crate::ui_format!("{} 个已有配置", "{} existing configurations", s.chat.external_cli_agents.len()),
                            &theme,
                        ),
                    ],
                    Section::Computer => vec![
                        div().child(self.computer.clone()),
                        self.text_field("浏览器扩展 Token", "/chatTools/playwrightExtensionToken", "从 Playwright 扩展复制 Token，供已启用的浏览器工具连接扩展。凭据仅保存到本机。", &theme, cx),
                    ],
                    Section::Hooks => {
                        vec![value("工作流 Hooks", "原生规则编辑与运行尚未接入", &theme)]
                    }
                    Section::Search => vec![
                        pending("启用网络搜索", s.lens.web_search.enabled, &theme),
                        value(
                            "搜索引擎",
                            format!("{:?}", s.lens.web_search.provider),
                            &theme,
                        ),
                        value(
                            "结果数量",
                            s.lens.web_search.max_results.to_string(),
                            &theme,
                        ),
                        value("搜索深度", s.lens.web_search.search_depth.clone(), &theme),
                    ],
                    Section::Usage => vec![value("模型调用统计", "原生统计视图尚未接入", &theme)],
                    Section::Backup => vec![
                        value("本地备份与恢复", "原生备份入口尚未接入", &theme),
                        value("WebDAV", "原生同步入口尚未接入", &theme),
                    ],
                    Section::About => vec![
                        div().child(self.model_database.clone()),
                        value("应用", "Kivio Desktop", &theme),
                        value("开发者", "ZM", &theme),
                        value("原生预览版本", "0.1.0 · GPUI Kit 0.8.0", &theme),
                        row("项目主页", Button::new("about-github").small().label(crate::i18n::text("在浏览器中打开")).on_click(|_, _, cx| cx.open_url("https://github.com/ZMGID/kivio")).into_any_element(), &theme),
                        row("版本发布", Button::new("about-releases").small().label(crate::i18n::text("查看发布版本")).on_click(|_, _, cx| cx.open_url("https://github.com/ZMGID/kivio/releases")).into_any_element(), &theme),
                        row("原生数据目录", Button::new("about-data").small().label(crate::i18n::text("在 Finder 中显示")).on_click(cx.listener(|this, _, _, cx| cx.reveal_path(&this.directory))).into_any_element(), &theme),
                        div().py_3().text_size(typography::ui_rems(12.)).text_color(theme.text_muted).child(crate::i18n::text("原生预览使用独立数据目录。发布页上的 Tauri 安装包不会升级当前 GPUI 预览。")),
                    ],
                    Section::Mixer => vec![],
                };
                body = body.child(card(section.label(), rows, &theme));
            }
            _ => {
                subtitle = "";
            }
        }
        div()
            .size_full()
            .track_focus(&self.focus)
            .child(page_frame(
                self.section.label(),
                subtitle,
                body.into_any_element(),
                cx,
            ))
            .into_any_element()
    }
}

impl SettingsPage {
    fn language(
        &self,
        field: TextPreference,
        current: &str,
        inherit: bool,
        cx: &Context<Self>,
    ) -> AnyElement {
        let options = if inherit {
            vec![
                ("", "跟随翻译语言"),
                ("zh", "中文"),
                ("zh-Hant", "繁體中文"),
                ("en", "English"),
            ]
        } else {
            vec![
                ("auto", "自动（中英互译）"),
                ("zh", "中文"),
                ("zh-Hant", "繁體中文"),
                ("en", "English"),
                ("ja", "日本語"),
                ("ko", "한국어"),
                ("fr", "Français"),
                ("de", "Deutsch"),
                ("es", "Español"),
                ("pt", "Português"),
                ("it", "Italiano"),
                ("ru", "Русский"),
                ("ar", "العربية"),
                ("hi", "हिन्दी"),
                ("th", "ไทย"),
                ("vi", "Tiếng Việt"),
                ("id", "Bahasa Indonesia"),
                ("ms", "Bahasa Melayu"),
                ("tr", "Türkçe"),
                ("nl", "Nederlands"),
                ("pl", "Polski"),
                ("uk", "Українська"),
                ("sv", "Svenska"),
                ("da", "Dansk"),
                ("no", "Norsk"),
                ("fi", "Suomi"),
            ]
        };
        let label = options
            .iter()
            .find(|(value, _)| *value == current)
            .map(|(_, label)| *label)
            .unwrap_or(current)
            .to_owned();
        let current = current.to_owned();
        let owner = cx.weak_entity();
        Button::new(SharedString::from(format!("language-{field:?}")))
            .small()
            .label(crate::i18n::display(label))
            .dropdown_caret(true)
            .disabled(self.busy)
            .dropdown_menu(move |mut menu, _, _| {
                for (value, label) in &options {
                    if field == TextPreference::ReadingLanguage && *value == "auto" {
                        continue;
                    }
                    let owner = owner.clone();
                    let value = value.to_string();
                    menu = menu.item(
                        PopupMenuItem::new(*label)
                            .checked(value == current)
                            .on_click(move |_, _, cx| {
                                let _ = owner.update(cx, |this, cx| {
                                    this.save_text(field, value.clone(), cx)
                                });
                            }),
                    );
                }
                menu
            })
            .into_any_element()
    }

    fn prompt_editor(
        &self,
        field: TextPreference,
        title: &'static str,
        hint: &'static str,
        theme: &Theme,
        cx: &Context<Self>,
    ) -> Div {
        let Some(input) = self.prompts.get(&field) else {
            return div();
        };
        let input = input.clone();
        let expanded = self.expanded_prompts.contains(&field);
        let editor = div()
            .flex()
            .flex_col()
            .gap_3()
            .pt_2()
            .child(
                Textarea::new(&input)
                    .disabled(self.busy)
                    .aria_label(title)
                    .h(rems(10.)),
            )
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new(SharedString::from(format!("reset-{field:?}")))
                            .small()
                            .label(crate::i18n::text("恢复默认"))
                            .disabled(self.busy)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                let Some(settings) = this.settings.as_ref() else {
                                    return;
                                };
                                let value = default_prompt(field, settings);
                                if let Some(input) = this.prompts.get(&field) {
                                    input.update(cx, |input, cx| {
                                        input.set_value(value.clone(), window, cx)
                                    });
                                }
                                this.save_text(field, value, cx);
                            })),
                    )
                    .child(
                        Button::new(SharedString::from(format!("save-{field:?}")))
                            .small()
                            .label(crate::i18n::text("保存提示词"))
                            .disabled(self.busy)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if let Some(input) = this.prompts.get(&field) {
                                    let value = input.read(cx).value().to_string();
                                    this.save_text(field, value, cx);
                                }
                            })),
                    ),
            )
            .child(self.field_feedback(&format!("{field:?}"), theme));
        div()
            .flex()
            .flex_col()
            .py_3()
            .gap_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_4()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(widgets::row_title(theme, title))
                            .child(
                                div()
                                    .mt_1()
                                    .text_size(typography::ui_rems(12.))
                                    .text_color(theme.text_muted)
                                    .child(hint),
                            ),
                    )
                    .child(
                        Button::new(SharedString::from(format!("edit-{field:?}")))
                            .small()
                            .label(if expanded { "收起" } else { "编辑…" })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if !this.expanded_prompts.remove(&field) {
                                    this.expanded_prompts.insert(field);
                                }
                                cx.notify();
                            })),
                    ),
            )
            .when(expanded, |view| {
                if crate::motion::reduced_motion(cx) {
                    view.child(editor)
                } else {
                    view.child(crate::motion::fade_quick(
                        SharedString::from(format!("prompt-entrance-{field:?}")),
                        editor,
                    ))
                }
            })
    }

    fn render_detail(&self, section: Section, theme: &Theme, cx: &Context<Self>) -> AnyElement {
        let s = self.settings.as_ref().unwrap();
        let mut body = div().flex().flex_col().gap_8();
        body = body.when(self.error.is_some(), |body| {
            body.child(self.error_notice(cx))
        });
        let subtitle;
        match section {
            Section::Translate => {
                subtitle = "输入翻译与快速翻译的语言、模型、OCR 和提示词。输入与选中文本翻译使用原生窗口；截图与替换翻译仍在接入。";
                body = body
                    .child(card("翻译语言", vec![
                        described_row("阅读目标语言", "你想看的语言，用于划词、截图和替换翻译。", self.language(TextPreference::ReadingLanguage, &s.reading_target_lang, false, cx), theme),
                        described_row("输入翻译输出语言", "你准备发送的语言；自动仅用于中英互译。", self.language(TextPreference::InputLanguage, &s.target_lang, false, cx), theme),
                    ], theme))
                    .child(card("输入翻译", vec![
                        self.model_pair("模型组合", "/translatorProviderId", "/translatorModel", false, theme, cx),
                        row("输入翻译窗口", Button::new("open-input-translation").small().label(crate::i18n::text("打开翻译")).disabled(self.busy).on_click(cx.listener(|this, _, _, cx| {
                            let result = crate::input_translation::open(this.directory.clone(), cx);
                            if let Err(error) = result { this.error = Some(error); cx.notify(); }
                        })).into_any_element(), theme),
                        self.prompt_editor(TextPreference::InputPrompt, "输入翻译提示词", "使用 {lang} 和 {text} 插入目标语言与原文；留空使用内置模板。", theme, cx),
                    ], theme))
                    .child(card("快速翻译 · 选项", vec![
                        row("快速翻译窗口", Button::new("open-selection-translation").small().label(crate::i18n::text("打开快速翻译…")).disabled(self.busy).on_click(cx.listener(|this, _, _, cx| {
                            if let Err(error) = crate::input_translation::open_selection(this.directory.clone(), String::new(), cx) { this.error = Some(error); cx.notify(); }
                        })).into_any_element(), theme),
                        self.toggle("启用快速翻译", "/screenshotTranslation/enabled", false, theme, cx),
                        self.toggle("显示原文", "/screenshotTranslation/directTranslate", true, theme, cx),
                        self.toggle("思考模式", "/screenshotTranslation/thinkingEnabled", false, theme, cx),
                        self.toggle("流式输出", "/screenshotTranslation/streamEnabled", false, theme, cx),
                        self.toggle("截图后保持全屏覆盖", "/screenshotTranslation/keepFullscreenAfterCapture", false, theme, cx),
                        self.text_field("翻译卡宽度", "/screenshotTranslation/cardWidth", "360–720 px", theme, cx),
                    ], theme))
                    .child(card("替换翻译", vec![
                        self.toggle("启用替换翻译", "/screenshotTranslation/replaceEnabled", false, theme, cx),
                        self.choice("替换翻译离线包", "/screenshotTranslation/rapidOcrTier", &[("standard", "标准 · PP-OCRv5"), ("high", "高精度 · PP-OCRv6")], theme, cx),
                    ], theme))
                    .child(self.offline.clone())
                    .child(card("截图翻译", vec![
                        self.choice("截图翻译处理方式", "/screenshotTranslation/ocrMode", &[("cloud_vision", "视觉模型"), ("system", "系统 OCR"), ("rapid_ocr", "RapidOCR")], theme, cx),
                        div().text_size(px(12.)).text_color(theme.text_muted).child(crate::i18n::text("截图翻译支持选区、窗口捕获与全局快捷键。替换翻译使用 RapidOCR，其原生交互仍在接入。")),
                        self.model_pair("模型组合", "/screenshotTranslation/providerId", "/screenshotTranslation/model", false, theme, cx),
                    ], theme))
                    .child(card("自定义提示词", vec![
                        self.prompt_editor(TextPreference::ScreenshotPrompt, "截图翻译提示词", "留空使用内置截图翻译模板。", theme, cx),
                        self.prompt_editor(TextPreference::TextPrompt, "选中文本翻译提示词", "独立于截图模板，适用于干净的选中文本。", theme, cx),
                        self.prompt_editor(TextPreference::ReplacementPrompt, "替换翻译提示词", "仅自定义翻译规则，保留原有 JSON 输出约定。", theme, cx),
                    ], theme));
            }
            Section::Lens => {
                subtitle =
                    "视觉问答的响应方式、对话与提示词。截图后可在浮窗追问，或发送到 Kivio Agent。";
                body = body
                    .child(card(
                        "Lens 配置",
                        vec![
                            self.toggle("启用 Lens", "/lens/enabled", false, theme, cx),
                            div().child(Button::new("try-native-lens").small().label(crate::i18n::text("打开 Lens 截图"))
                                .disabled(self.busy || !s.lens.enabled)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    let root = this.directory.clone(); let owner = cx.entity().downgrade();
                                    cx.defer(move |cx| { if let Err(error) = crate::native_capture::toggle_lens(root, cx) {
                                        let _ = owner.update(cx, |this, cx| { this.error = Some(error); cx.notify(); });
                                    }});
                                }))),
                            row(
                                "响应语言",
                                self.language(
                                    TextPreference::LensLanguage,
                                    &s.lens.default_language,
                                    true,
                                    cx,
                                ),
                                theme,
                            ),
                            self.toggle("流式输出", "/lens/streamEnabled", false, theme, cx),
                            self.toggle("思考模式", "/lens/thinkingEnabled", false, theme, cx),
                        ],
                        theme,
                    ))
                    .child(card(
                        "对话",
                        vec![
                            self.toggle("发送到 Kivio Agent", "/lens/sendToChat", false, theme, cx),
                            self.choice(
                                "消息顺序",
                                "/lens/messageOrder",
                                &[("asc", "从旧到新"), ("desc", "从新到旧")],
                                theme,
                                cx,
                            ),
                            self.toggle("截图选择提示", "/lens/showCaptureHint", false, theme, cx),
                        ],
                        theme,
                    ))
                    .child(card(
                        "翻译引擎",
                        vec![self.model_pair(
                            "模型组合",
                            "/lens/providerId",
                            "/lens/model",
                            true,
                            theme,
                            cx,
                        )],
                        theme,
                    ))
                    .child(card(
                        "截图自动归档",
                        vec![
                            self.toggle("截图自动归档", "/imageArchiveEnabled", false, theme, cx),
                            self.text_field(
                                "归档路径",
                                "/imageArchivePath",
                                "留空时不自动归档。选择目录后开启截图自动归档。",
                                theme,
                                cx,
                            ),
                        ],
                        theme,
                    ))
                    .child(card(
                        "自定义提示词",
                        vec![
                            self.prompt_editor(
                                TextPreference::LensSystemPrompt,
                                "系统提示词",
                                "留空使用原有 Lens 系统模板。",
                                theme,
                                cx,
                            ),
                            self.prompt_editor(
                                TextPreference::LensQuestionPrompt,
                                "提问提示词",
                                "留空使用原有 Lens 提问模板。",
                                theme,
                                cx,
                            ),
                        ],
                        theme,
                    ));
            }
            Section::Chat => {
                subtitle = "个人资料、Kivio Agent 与 Kivio Chat。";
                if let Some(name) = &self.name {
                    body = body.child(card(
                        "个人资料",
                        vec![row(
                            "用户名",
                            div()
                                .flex()
                                .w(rems(20.))
                                .gap_2()
                                .child(
                                    div().flex_1().min_w_0().child(
                                        Input::new(name)
                                            .small()
                                            .aria_label("用户名")
                                            .disabled(self.busy),
                                    ),
                                )
                                .child(
                                    Button::new("save-display-name")
                                        .small()
                                        .label(crate::i18n::text("保存"))
                                        .disabled(self.busy)
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            if let Some(name) = &this.name {
                                                let value = name.read(cx).value().to_string();
                                                this.save_text(TextPreference::UserName, value, cx);
                                            }
                                        })),
                                )
                                .into_any_element(),
                            theme,
                        )],
                        theme,
                    ));
                }
                body = body
                    .child(card(
                        "Kivio Agent",
                        vec![
                            self.text_field(
                                "普通对话工作目录",
                                "/chatTools/nativeTools/workingDirectory",
                                "项目对话和导入 CLI 对话保留各自的工作目录。",
                                theme,
                                cx,
                            ),
                            value(
                                "最大输出 Token",
                                self.providers
                                    .read(cx)
                                    .selection()
                                    .map(|(provider, model)| {
                                        format!(
                                            "{} · {}",
                                            kivio_model_runtime::request_max_tokens(
                                                &provider, &model
                                            ),
                                            model
                                        )
                                    })
                                    .unwrap_or_else(|_| "请先选择对话模型".into()),
                                theme,
                            ),
                            self.toggle("流式输出", "/chat/streamEnabled", false, theme, cx),
                            self.toggle("思考模式", "/chat/thinkingEnabled", false, theme, cx),
                            row(
                                "响应语言",
                                self.language(
                                    TextPreference::AgentLanguage,
                                    &s.chat.default_language,
                                    true,
                                    cx,
                                ),
                                theme,
                            ),
                            self.prompt_editor(
                                TextPreference::AgentPrompt,
                                "Agent 系统提示词",
                                "定义身份与风格；留空恢复内置模板。",
                                theme,
                                cx,
                            ),
                        ],
                        theme,
                    ))
                    .child(card(
                        "工具与扩展",
                        vec![
                            value(
                                "记忆",
                                if s.chat_memory.enabled {
                                    "已配置"
                                } else {
                                    "未启用"
                                },
                                theme,
                            ),
                            value(
                                "本地 CLI",
                                crate::ui_format!("{} 个配置", "{} configurations", s.chat.external_cli_agents.len()),
                                theme,
                            ),
                            div().text_size(px(12.)).text_color(theme.text_muted).child(crate::i18n::text("Agent 根据当前模型、工具开关与审批规则执行工具；运行状态与结果显示在对话中。"),
                            ),
                        ],
                        theme,
                    ))
                    .child(card(
                        "Kivio Chat",
                        vec![
                            self.prompt_editor(
                                TextPreference::ChatPrompt,
                                "Chat 附加指令",
                                "可选。与 Agent 系统提示词独立保存。",
                                theme,
                                cx,
                            ),
                            self.toggle("联网搜索", "/chat/chatMode/webSearch", false, theme, cx),
                            self.toggle("读取网页", "/chat/chatMode/webFetch", false, theme, cx),
                            self.toggle(
                                "知识库检索",
                                "/chat/chatMode/knowledgeSearch",
                                false,
                                theme,
                                cx,
                            ),
                            self.toggle("记忆只读", "/chat/chatMode/memoryTools", false, theme, cx),
                            self.toggle("只读 MCP", "/chat/chatMode/mcpReadOnly", false, theme, cx),
                        ],
                        theme,
                    ));
            }
            _ => unreachable!(),
        }
        page_frame(section.label(), subtitle, body.into_any_element(), cx)
    }
}

fn described_row(
    label: &'static str,
    description: &'static str,
    control: AnyElement,
    theme: &Theme,
) -> Div {
    div()
        .flex()
        .flex_wrap()
        .items_center()
        .justify_between()
        .gap_4()
        .py_3()
        .min_h(px(60.))
        .child(
            div().flex_1().min_w(rems(12.)).child(crate::i18n::display(label)).child(
                div()
                    .mt_1()
                    .text_size(px(12.))
                    .text_color(theme.text_muted)
                    .child(crate::i18n::display(description)),
            ),
        )
        .child(div().flex_none().child(control))
}

fn default_prompt(field: TextPreference, settings: &Settings) -> String {
    use kivio_model_runtime::{prompts, settings_config};
    match field {
        TextPreference::InputPrompt => prompts::DEFAULT_TRANSLATION_TEMPLATE.into(),
        TextPreference::ScreenshotPrompt => prompts::DEFAULT_SCREENSHOT_TRANSLATION_TEMPLATE.into(),
        TextPreference::TextPrompt => prompts::DEFAULT_SELECTED_TEXT_TRANSLATION_TEMPLATE.into(),
        TextPreference::ReplacementPrompt => prompts::DEFAULT_REPLACE_TRANSLATION_TEMPLATE.into(),
        TextPreference::LensSystemPrompt => settings_config::default_lens_system_prompt(
            if settings.lens.default_language.is_empty() {
                &settings.reading_target_lang
            } else {
                &settings.lens.default_language
            },
            true,
        ),
        TextPreference::LensQuestionPrompt => settings_config::default_question_prompt(
            if settings.lens.default_language.is_empty() {
                &settings.reading_target_lang
            } else {
                &settings.lens.default_language
            },
            true,
        ),
        TextPreference::AgentPrompt => settings_config::default_chat_system_prompt(false),
        _ => String::new(),
    }
}

fn theme_library_grid(
    editor: Entity<crate::settings::custom_themes::CustomThemes>,
    cx: &App,
) -> Div {
    let registry = zeron_theme::ThemeRegistry::builtin();
    let current = crate::appearance::themes(cx);
    let mut grid = div().flex().flex_wrap().gap_3();
    for (id, label) in [
        ("neutral", "中性"),
        ("warm", "暖白"),
        ("cool", "冷白"),
        ("graphite", "石墨"),
        ("blossom", "樱粉"),
        ("grove", "森林"),
        ("ocean", "海湾"),
        ("ember", "余烬"),
        ("iris", "鸢尾"),
        ("white", "纯白"),
        ("nord", "北欧 Nord"),
        ("solarized", "日晒 Solarized"),
    ] {
        let light = format!("kivio-{id}-light");
        let dark = format!("kivio-{id}-dark");
        let selected = current.variant_id(zeron_theme::Appearance::Light) == light
            && current.variant_id(zeron_theme::Appearance::Dark) == dark;
        let previews = div()
            .flex()
            .h(px(76.))
            .w_full()
            .child(theme_miniature(registry.variant(&light).unwrap()))
            .child(theme_miniature(registry.variant(&dark).unwrap()));
        let editor = editor.clone();
        let editing = editor.read(cx).editing();
        grid = grid.child(
            div()
                .w(px(180.))
                .flex()
                .flex_col()
                .gap_2()
                .child(previews)
                .child(
                    Button::new(SharedString::from(format!("theme-{id}")))
                        .small()
                        .label(if selected {
                            crate::ui_format!("{label} · 已应用", "{label} · Applied")
                        } else {
                            label.to_string()
                        })
                        .selected(selected)
                        .disabled(editing)
                        .w_full()
                        .on_click(move |_, _, cx| {
                            editor.update(cx, |editor, cx| editor.apply_builtin(id, cx));
                        }),
                ),
        );
    }
    div().flex().flex_col().gap_3().child(crate::i18n::text("主题库")).child(grid)
}
pub(crate) fn theme_miniature(variant: &zeron_theme::ThemeVariant) -> Div {
    let color = |color: zeron_theme::Color| -> Hsla {
        rgba(
            ((color.r as u32) << 24)
                | ((color.g as u32) << 16)
                | ((color.b as u32) << 8)
                | color.a as u32,
        )
        .into()
    };
    let colors = &variant.colors;
    div()
        .flex()
        .flex_1()
        .h_full()
        .border_1()
        .border_color(color(colors.border))
        .rounded(px(6.))
        .overflow_hidden()
        .child(div().w(px(22.)).h_full().bg(color(colors.shell)))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .gap(px(8.))
                .p(px(8.))
                .bg(color(colors.background))
                .child(
                    div()
                        .h(px(4.))
                        .w(px(32.))
                        .rounded(px(2.))
                        .bg(color(colors.text_muted)),
                )
                .child(
                    div()
                        .h(px(12.))
                        .w_full()
                        .rounded(px(3.))
                        .bg(color(colors.raised)),
                )
                .child(
                    div()
                        .h(px(8.))
                        .w(px(24.))
                        .rounded(px(3.))
                        .bg(color(variant.accent.primary)),
                ),
        )
}

#[cfg(target_os = "macos")]
fn permission_status() -> (bool, bool) {
    use kivio_model_runtime::platform_permissions::*;
    (
        check_accessibility(false),
        check_screen_recording_permission(),
    )
}
