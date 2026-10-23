//! Native provider editing and its durable configuration snapshot.
//! Provider wire types and normalization are owned by the shared Rust backend.
#[path = "providers/oauth.rs"]
mod oauth;
#[path = "providers/purposes.rs"]
mod purposes;
use crate::provider_settings::{
    DefaultModelsConfig, ModelProvider, ProviderRequestConfig, sanitize_provider,
};
use gpui_kit::component::{
    ActiveTheme, Disableable, Selectable, StyledExt, WindowExt,
    button::{Button, ButtonVariants},
    checkbox::Checkbox,
    input::{Input, InputEvent, InputState},
    menu::{DropdownMenu, PopupMenuItem},
    notification::Notification,
    select::{Select, SelectEvent, SelectItem, SelectState},
};
use gpui_kit::{prelude::FluentBuilder, *};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

const FORMATS: [(&str, &str); 5] = [
    ("OpenAI Chat Completions", "openai_chat"),
    ("Anthropic Messages", "anthropic_messages"),
    ("OpenAI Responses", "openai_responses"),
    ("Google Gemini", "gemini"),
    ("xAI Responses", "xai_responses"),
];

// Windows share this writer. Keep the baseline check and atomic replacement in
// one critical section so two live settings entities cannot both commit stale data.
static PROVIDER_WRITES: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub(crate) enum ProviderEvent {
    Changed,
}

#[derive(Clone)]
pub(crate) struct ModelChoice {
    pub(crate) provider_id: String,
    pub(crate) model: String,
    pub(crate) label: SharedString,
    value: (String, String),
}

impl SelectItem for ModelChoice {
    type Value = (String, String);
    fn title(&self) -> SharedString {
        self.label.clone()
    }
    fn value(&self) -> &Self::Value {
        &self.value
    }
}

// Keep unrelated settings fields intact, including other default-model purposes.
// The original document is also the confirmed write baseline; a failed save never
// advances it or replaces the UI draft with an unconfirmed result.
#[derive(Clone)]
struct ProviderDocument {
    raw: Value,
    baseline: Option<Vec<u8>>,
    providers: Vec<ModelProvider>,
    selection: (String, String),
    defaults: DefaultModelsConfig,
}

impl Default for ProviderDocument {
    fn default() -> Self {
        Self {
            raw: json!({}),
            baseline: None,
            providers: vec![],
            selection: Default::default(),
            defaults: Default::default(),
        }
    }
}

pub(crate) fn load_selection(
    path: &Path,
    selection: Option<(&str, &str)>,
) -> Result<(ModelProvider, String), String> {
    let document = ProviderDocument::load(path)?;
    let (provider, model) = selection.unwrap_or((&document.selection.0, &document.selection.1));
    resolve_selection(&document.providers, provider, model)
}

fn resolve_selection(
    providers: &[ModelProvider],
    provider_id: &str,
    model: &str,
) -> Result<(ModelProvider, String), String> {
    let provider = providers
        .iter()
        .find(|p| p.id == provider_id && p.enabled)
        .ok_or_else(|| "请在供应商设置中添加并启用供应商，然后选择模型。".to_string())?;
    if model.trim().is_empty()
        || !provider
            .enabled_models
            .iter()
            .any(|candidate| candidate == model)
    {
        return Err("此模型尚未启用，请在供应商设置中启用后重试。".into());
    }
    Ok((provider.clone(), model.to_owned()))
}

impl ProviderDocument {
    fn load(path: &Path) -> Result<Self, String> {
        let Some(bytes) = read_document(path)? else {
            return Ok(Self::default());
        };
        let raw: Value = serde_json::from_slice(&bytes)
            .map_err(|_| "供应商配置格式损坏，请先修复配置文件再重试。".to_string())?;
        if !raw.is_object() {
            return Err("供应商配置必须是 JSON 对象。".into());
        }
        let mut providers: Vec<ModelProvider> =
            serde_json::from_value(raw.get("providers").cloned().unwrap_or_else(|| json!([])))
                .map_err(|_| "无法读取供应商配置，请检查配置文件。".to_string())?;
        for provider in &mut providers {
            sanitize_provider(provider);
        }
        let selection = raw.pointer("/defaultModels/chat");
        let explicit_id = selection
            .and_then(|value| value.get("providerId"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let (provider, model) = if !explicit_id.is_empty() {
            (
                explicit_id,
                selection
                    .and_then(|value| value.get("model"))
                    .and_then(Value::as_str)
                    .unwrap_or(""),
            )
        } else {
            (
                raw.get("chatProviderId")
                    .and_then(Value::as_str)
                    .unwrap_or(""),
                raw.get("chatModel").and_then(Value::as_str).unwrap_or(""),
            )
        };
        let selection = (provider.to_owned(), model.to_owned());
        let mut defaults: DefaultModelsConfig = serde_json::from_value(
            raw.get("defaultModels")
                .cloned()
                .unwrap_or_else(|| json!({})),
        )
        .map_err(|_| "无法读取模型用途配置，请检查配置文件。".to_string())?;
        defaults.chat.provider_id = selection.0;
        defaults.chat.model = selection.1;
        purposes::sanitize(&mut defaults, &providers);
        let selection = (
            defaults.chat.provider_id.clone(),
            defaults.chat.model.clone(),
        );
        Ok(Self {
            raw,
            baseline: Some(bytes),
            providers,
            selection,
            defaults,
        })
    }

    fn save(mut self, path: &Path) -> Result<Self, String> {
        let _writer = PROVIDER_WRITES
            .lock()
            .map_err(|_| "供应商配置写入器不可用，请重新启动后重试。".to_string())?;
        if read_document(path)? != self.baseline {
            return Err(
                "供应商配置已被其他窗口或程序修改。请重新读取后再保存；当前编辑内容仍保留。".into(),
            );
        }
        for provider in &mut self.providers {
            sanitize_provider(provider);
        }
        let mut persisted = self.providers.clone();
        for provider in &mut persisted {
            provider.api_key_legacy = provider.preferred_api_key().map(str::to_owned);
        }
        self.raw["providers"] =
            serde_json::to_value(persisted).map_err(|_| "无法编码供应商配置。".to_string())?;
        if !self.raw.get("defaultModels").is_some_and(Value::is_object) {
            self.raw["defaultModels"] = json!({});
        }
        self.defaults.chat.provider_id = self.selection.0.clone();
        self.defaults.chat.model = self.selection.1.clone();
        purposes::sanitize(&mut self.defaults, &self.providers);
        self.selection = (
            self.defaults.chat.provider_id.clone(),
            self.defaults.chat.model.clone(),
        );
        self.raw["chatProviderId"] = json!(self.selection.0);
        self.raw["chatModel"] = json!(self.selection.1);
        let defaults = serde_json::to_value(&self.defaults)
            .map_err(|_| "无法编码模型用途配置。".to_string())?;
        for (key, value) in defaults
            .as_object()
            .expect("default models serialize as an object")
        {
            self.raw["defaultModels"][key] = value.clone();
        }
        let content = serde_json::to_string_pretty(&self.raw)
            .map_err(|_| "无法编码供应商配置。".to_string())?;
        crate::file_io::atomic_write(path, &content, "providers")
            .map_err(|_| "保存供应商配置失败。请检查数据目录权限，然后重试。".to_string())?;
        self.baseline = Some(content.into_bytes());
        Ok(self)
    }
}

fn read_document(path: &Path) -> Result<Option<Vec<u8>>, String> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(_) => Err("无法读取供应商配置。请检查数据目录权限，然后重试。".into()),
    }
}

pub(crate) struct ProviderSettings {
    mixer_only: bool,
    path: PathBuf,
    confirmed: ProviderDocument,
    draft: Vec<ModelProvider>,
    editing: Option<String>,
    name: Entity<InputState>,
    endpoint: Entity<InputState>,
    key: Entity<InputState>,
    models_input: Entity<InputState>,
    format: Entity<SelectState<Vec<String>>>,
    model: Entity<SelectState<Vec<ModelChoice>>>,
    dirty: bool,
    ready: bool,
    busy: bool,
    error: Option<String>,
    task: Option<Task<()>>,
    oauth_run: Option<oauth::OAuthRun>,
    oauth_login: Option<kivio_model_runtime::provider_oauth::Login>,
    oauth_account: Option<kivio_model_runtime::provider_oauth::account::AccountIdentity>,
    oauth_usage: Option<kivio_model_runtime::provider_oauth::usage::AccountUsage>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<ProviderEvent> for ProviderSettings {}

impl ProviderSettings {
    pub(crate) fn set_mixer_only(&mut self, value: bool) {
        self.mixer_only = value;
    }
    pub(crate) fn new(path: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let name = cx.new(|cx| InputState::new(window, cx).placeholder("供应商名称"));
        let endpoint =
            cx.new(|cx| InputState::new(window, cx).placeholder("https://api.example.com/v1"));
        let key = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("输入新密钥；留空保留已保存的密钥")
                .masked(true)
        });
        let models_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("输入模型 ID，多个模型用逗号分隔"));
        let format = cx.new(|cx| {
            SelectState::new(
                FORMATS
                    .iter()
                    .map(|(label, _)| label.to_string())
                    .collect::<Vec<_>>(),
                None,
                window,
                cx,
            )
        });
        let model = cx.new(|cx| {
            SelectState::new(Vec::<ModelChoice>::new(), None, window, cx).searchable(true)
        });
        let mut subscriptions = Vec::new();
        for input in [&name, &endpoint, &key, &models_input] {
            subscriptions.push(cx.subscribe(input, |this, _, event, cx| {
                if matches!(event, InputEvent::Change) {
                    this.dirty = true;
                    this.error = None;
                    cx.notify();
                }
            }));
        }
        subscriptions.push(
            cx.subscribe(&format, |this, _, _: &SelectEvent<Vec<String>>, cx| {
                this.dirty = true;
                this.error = None;
                cx.notify();
            }),
        );
        subscriptions.push(
            cx.subscribe_in(&model, window, |this, _, event, window, cx| {
                if let SelectEvent::Confirm(Some((provider, model))) = event {
                    this.select_model(provider.clone(), model.clone(), window, cx);
                }
            }),
        );
        let mut settings = Self {
            mixer_only: false,
            path,
            confirmed: ProviderDocument::default(),
            draft: vec![],
            editing: None,
            name,
            endpoint,
            key,
            models_input,
            format,
            model,
            dirty: false,
            ready: false,
            busy: false,
            error: None,
            task: None,
            oauth_run: None,
            oauth_login: None,
            oauth_account: None,
            oauth_usage: None,
            _subscriptions: subscriptions,
        };
        settings.reload(window, cx);
        settings
    }

    pub(crate) fn selection(&self) -> Result<(ModelProvider, String), String> {
        let chat = &self.default_models().chat;
        self.selected(&chat.provider_id, &chat.model)
    }

    pub(crate) fn default_models(&self) -> &DefaultModelsConfig {
        &self.confirmed.defaults
    }

    pub(crate) fn selected(
        &self,
        provider_id: &str,
        model: &str,
    ) -> Result<(ModelProvider, String), String> {
        if !self.ready {
            return Err("供应商配置尚未读取，请在设置中重试。".into());
        }
        resolve_selection(&self.confirmed.providers, provider_id, model)
    }

    pub(crate) fn models(&self) -> Vec<ModelChoice> {
        self.confirmed
            .providers
            .iter()
            .filter(|provider| provider.enabled)
            .flat_map(|provider| {
                provider
                    .enabled_models
                    .iter()
                    .map(move |model| ModelChoice {
                        provider_id: provider.id.clone(),
                        model: model.clone(),
                        label: format!("{} · {model}", provider.name).into(),
                        value: (provider.id.clone(), model.clone()),
                    })
            })
            .collect()
    }

    pub(crate) fn select_model(
        &mut self,
        provider_id: String,
        model: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy || !self.ready || self.dirty {
            window.push_notification(
                Notification::error("请先完成供应商配置的读取、保存或放弃编辑，再切换默认模型。"),
                cx,
            );
            self.sync_model(window, cx);
            return;
        }
        if let Err(error) = self.selected(&provider_id, &model) {
            self.error = Some(error);
            self.sync_model(window, cx);
            cx.notify();
            return;
        }
        let mut next = self.confirmed.clone();
        next.selection = (provider_id, model);
        self.persist(next, window, cx);
    }

    fn sync_model(&self, window: &mut Window, cx: &mut Context<Self>) {
        let items = self.models();
        self.model.update(cx, |state, cx| {
            state.set_items(items, window, cx);
            state.set_selected_value(&self.confirmed.selection, window, cx);
        });
    }

    fn reload(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        let path = self.path.clone();
        let work = cx.background_spawn(async move { ProviderDocument::load(&path) });
        self.task = Some(cx.spawn_in(window, async move |owner, cx| {
            let result = work.await;
            let _ = owner.update_in(cx, |this, window, cx| {
                this.busy = false;
                match result {
                    Ok(document) => this.accept(document, window, cx),
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn accept(&mut self, document: ProviderDocument, window: &mut Window, cx: &mut Context<Self>) {
        self.confirmed = document;
        self.draft = self.confirmed.providers.clone();
        self.ready = true;
        self.dirty = false;
        self.error = None;
        let editing = self
            .editing
            .clone()
            .filter(|id| self.draft.iter().any(|provider| &provider.id == id))
            .or_else(|| self.draft.first().map(|provider| provider.id.clone()));
        self.open_editor(editing, window, cx);
        self.sync_model(window, cx);
        cx.emit(ProviderEvent::Changed);
    }

    fn persist(&mut self, next: ProviderDocument, window: &mut Window, cx: &mut Context<Self>) {
        self.busy = true;
        self.error = None;
        let path = self.path.clone();
        let work = cx.background_spawn(async move { next.save(&path) });
        self.task = Some(cx.spawn_in(window, async move |owner, cx| {
            let result = work.await;
            let _ = owner.update_in(cx, |this, window, cx| {
                this.busy = false;
                match result {
                    Ok(document) => this.accept(document, window, cx),
                    Err(error) => {
                        this.error = Some(error.clone());
                        this.sync_model(window, cx);
                        window.push_notification(Notification::error(error), cx);
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn open_editor(&mut self, id: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        self.editing = id;
        self.oauth_account = None;
        self.oauth_usage = None;
        let provider = self
            .draft
            .iter()
            .find(|provider| Some(&provider.id) == self.editing.as_ref());
        for (input, value) in [
            (
                &self.name,
                provider.map(|p| p.name.clone()).unwrap_or_default(),
            ),
            (
                &self.endpoint,
                provider.map(|p| p.base_url.clone()).unwrap_or_default(),
            ),
            (
                &self.models_input,
                provider
                    .map(|p| p.enabled_models.join(", "))
                    .unwrap_or_default(),
            ),
            (&self.key, String::new()),
        ] {
            input.update(cx, |state, cx| state.set_value(value, window, cx));
        }
        let format = provider
            .map(|p| p.api_format.as_str())
            .unwrap_or("openai_chat");
        let label = FORMATS
            .iter()
            .find(|(_, value)| *value == format)
            .unwrap_or(&FORMATS[0])
            .0
            .to_owned();
        self.format
            .update(cx, |state, cx| state.set_selected_value(&label, window, cx));
    }

    fn capture_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(provider) = self
            .draft
            .iter_mut()
            .find(|provider| Some(&provider.id) == self.editing.as_ref())
        else {
            return;
        };
        provider.name = self.name.read(cx).value().trim().to_owned();
        provider.base_url = self.endpoint.read(cx).value().trim().to_owned();
        let label = self
            .format
            .read(cx)
            .selected_value()
            .map(String::as_str)
            .unwrap_or(FORMATS[0].0);
        provider.api_format = FORMATS
            .iter()
            .find(|(title, _)| *title == label)
            .unwrap_or(&FORMATS[0])
            .1
            .to_owned();
        let models = self.models_input.read(cx).value();
        provider.enabled_models.clear();
        for model in models
            .split([',', '，', '\n'])
            .map(str::trim)
            .filter(|model| !model.is_empty())
        {
            if !provider.enabled_models.iter().any(|value| value == model) {
                provider.enabled_models.push(model.to_owned());
            }
            if !provider.available_models.iter().any(|value| value == model) {
                provider.available_models.push(model.to_owned());
            }
        }
        let key = self.key.read(cx).value();
        let key = key.trim();
        if !key.is_empty() {
            let index = provider.clamped_active_key_index();
            if let Some(slot) = provider.api_keys.get_mut(index) {
                *slot = key.to_owned();
            } else {
                provider.api_keys.push(key.to_owned());
            }
            self.key
                .update(cx, |state, cx| state.set_value("", window, cx));
        }
    }

    fn add_provider(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || !self.ready {
            return;
        }
        self.capture_editor(window, cx);
        let id = uuid::Uuid::new_v4().to_string();
        self.draft.push(ModelProvider {
            id: id.clone(),
            name: "新供应商".into(),
            api_keys: vec![],
            api_key_legacy: None,
            base_url: String::new(),
            available_models: vec![],
            enabled_models: vec![],
            enabled: true,
            api_format: "openai_chat".into(),
            model_overrides: HashMap::new(),
            compress_request_body: false,
            request: ProviderRequestConfig::default(),
            active_key_index: 0,
        });
        self.open_editor(Some(id), window, cx);
        self.name.update(cx, |state, cx| state.focus(window, cx));
        self.dirty = true;
        cx.notify();
    }

    fn add_key(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let key = self.key.read(cx).value().trim().to_owned();
        if key.is_empty() {
            self.error = Some("请先输入要添加的新密钥。".into());
            cx.notify();
            return;
        }
        // Capture the other fields without replacing the currently active key.
        self.key
            .update(cx, |state, cx| state.set_value("", window, cx));
        self.capture_editor(window, cx);
        if let Some(provider) = self
            .draft
            .iter_mut()
            .find(|provider| Some(&provider.id) == self.editing.as_ref())
        {
            provider.active_key_index = provider
                .api_keys
                .iter()
                .position(|value| value == &key)
                .unwrap_or_else(|| {
                    provider.api_keys.push(key);
                    provider.api_keys.len() - 1
                });
        }
        self.dirty = true;
        self.error = None;
        cx.notify();
    }

    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || !self.ready {
            return;
        }
        self.capture_editor(window, cx);
        for provider in &self.draft {
            if provider.name.is_empty() {
                self.error = Some("请填写供应商名称。".into());
            } else if !(provider.base_url.starts_with("https://")
                || provider.base_url.starts_with("http://"))
            {
                self.error = Some("API 地址需要以 https:// 或 http:// 开头。".into());
            } else {
                continue;
            }
            self.open_editor(Some(provider.id.clone()), window, cx);
            cx.notify();
            return;
        }
        let mut next = self.confirmed.clone();
        next.providers = self.draft.clone();
        self.persist(next, window, cx);
    }
}

fn field(label: &'static str, control: impl IntoElement) -> impl IntoElement {
    div()
        .v_flex()
        .gap_2()
        .child(div().text_sm().child(label))
        .child(control)
}

impl Render for ProviderSettings {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let disabled = self.busy || !self.ready;
        let mut content = div().v_flex().w_full().min_w_0().gap_4();
        if !self.ready {
            return content
                .child(div().text_sm().child(if self.busy {
                    "正在读取供应商…"
                } else {
                    "供应商配置未能读取。"
                }))
                .when_some(self.error.clone(), |view, error| {
                    view.child(div().text_sm().text_color(cx.theme().danger).child(error))
                })
                .child(
                    Button::new("providers-reload")
                        .label("重新读取")
                        .disabled(self.busy)
                        .on_click(cx.listener(|this, _, window, cx| this.reload(window, cx))),
                )
                .into_any_element();
        }
        if self.mixer_only {
            return self.render_purposes(cx);
        }
        // ProvidersTab owns a collection/detail layout. Keep the native editor's
        // retained inputs and durable writer while restoring that navigation.
        let mut providers = div().v_flex().gap_2().child(
            Button::new("provider-add")
                .label("添加供应商")
                .primary()
                .w_full()
                .disabled(disabled)
                .on_click(cx.listener(|this, _, window, cx| this.add_provider(window, cx))),
        );
        for provider in &self.draft {
            let id = provider.id.clone();
            providers = providers.child(
                Button::new(SharedString::from(format!("provider-{}", provider.id)))
                    .label(if provider.enabled {
                        provider.name.clone()
                    } else {
                        format!("{} · 已停用", provider.name)
                    })
                    .ghost()
                    .w_full().justify_start()
                    .selected(self.editing.as_ref() == Some(&provider.id))
                    .disabled(disabled)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.capture_editor(window, cx);
                        this.open_editor(Some(id.clone()), window, cx);
                        cx.notify();
                    })),
            );
        }

        if let Some(provider) = self
            .draft
            .iter()
            .find(|provider| Some(&provider.id) == self.editing.as_ref())
        {
            let provider_id = provider.id.clone();
            let oauth = provider.request.oauth.is_some();
            let signed = provider
                .request
                .oauth
                .as_ref()
                .and_then(|auth| auth.credential_id.as_ref())
                .is_some();
            let key_count = provider.api_keys.len();
            let active_key = provider.clamped_active_key_index();
            let auth_owner = cx.weak_entity();
            let auth_kind = provider
                .request
                .oauth
                .as_ref()
                .map(|auth| auth.provider.clone());
            let available_models = provider.available_models.clone();
            let enabled_models = provider.enabled_models.clone();
            let model_owner = cx.weak_entity();
            content = content
                .child(field(
                    "名称",
                    Input::new(&self.name)
                        .id("provider-name")
                        .aria_label("供应商名称")
                        .disabled(disabled),
                ))
                .child(field(
                    "认证方式",
                    Button::new("provider-auth-method")
                        .dropdown_caret(true)
                        .label(match auth_kind.as_deref() {
                            Some("codex") => "Codex OAuth",
                            Some("kimi") => "Kimi OAuth",
                            Some("antigravity") => "Antigravity OAuth",
                            _ => "API Key",
                        })
                        .disabled(disabled || signed)
                        .tooltip(if signed {
                            "先断开账号再更改认证方式"
                        } else {
                            "选择认证方式"
                        })
                        .dropdown_menu(move |mut menu, _, _| {
                            for (label, kind) in [
                                ("API Key", None),
                                ("Codex OAuth", Some("codex")),
                                ("Kimi OAuth", Some("kimi")),
                                ("Antigravity OAuth", Some("antigravity")),
                            ] {
                                let owner = auth_owner.clone();
                                menu = menu.item(
                                    PopupMenuItem::new(label)
                                        .checked(auth_kind.as_deref() == kind)
                                        .on_click(move |_, window, cx| {
                                            let _ = owner.update(cx, |this, cx| {
                                                this.change_auth(kind, window, cx)
                                            });
                                        }),
                                );
                            }
                            menu
                        }),
                ))
                .child(field(
                    "接口地址",
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div().flex_1().min_w_0().child(
                                Input::new(&self.endpoint)
                                    .id("provider-endpoint")
                                    .aria_label("API 地址")
                                    .disabled(disabled || oauth),
                            ),
                        )
                        .child(
                            div().w(px(180.)).flex_none().child(
                                Select::new(&self.format)
                                    .id("provider-format")
                                    .disabled(disabled || oauth)
                                    .w_full(),
                            ),
                        ),
                ))
                .child(field(
                    "启用的模型",
                    Input::new(&self.models_input)
                        .id("provider-models")
                        .aria_label("启用的模型，多个模型用逗号分隔")
                        .disabled(disabled),
                ))
                .child(
                    Button::new("provider-model-catalog")
                        .label("从可用模型选择")
                        .dropdown_caret(true)
                        .disabled(disabled || available_models.is_empty())
                        .dropdown_menu(move |mut menu, _, _| {
                            for model in &available_models {
                                let owner = model_owner.clone();
                                let model = model.clone();
                                menu = menu.item(
                                    PopupMenuItem::new(model.clone())
                                        .checked(enabled_models.contains(&model))
                                        .on_click(move |_, window, cx| {
                                            let _ = owner.update(cx, |this, cx| {
                                                this.capture_editor(window, cx);
                                                if let Some(provider) =
                                                    this.draft.iter_mut().find(|value| {
                                                        Some(&value.id) == this.editing.as_ref()
                                                    })
                                                {
                                                    if provider.enabled_models.contains(&model) {
                                                        provider
                                                            .enabled_models
                                                            .retain(|value| value != &model);
                                                    } else {
                                                        provider.enabled_models.push(model.clone());
                                                    }
                                                    let models = provider.enabled_models.join(", ");
                                                    this.models_input.update(cx, |input, cx| {
                                                        input.set_value(models, window, cx)
                                                    });
                                                }
                                                this.dirty = true;
                                                cx.notify();
                                            });
                                        }),
                                );
                            }
                            menu
                        }),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child("填写供应商提供的模型 ID；已有模型参数覆盖会保留。"),
                );
            if oauth {
                content = content.child(self.render_oauth(provider, cx));
            } else {
                let key_owner = cx.weak_entity();
                content = content.child(field("API Key", Input::new(&self.key).id("provider-key").aria_label("新的 API Key").disabled(disabled)))
                    .child(div().flex().flex_wrap().items_center().gap_3()
                        .child(div().text_xs().text_color(cx.theme().muted_foreground).child(if key_count == 0 { "没有配置密钥；匿名端点可以留空。".to_string() } else { format!("当前配置包含 {key_count} 个密钥。输入新值会替换当前密钥。") }))
                        .child(Button::new("provider-active-key").label(format!("密钥 {}", active_key + 1)).dropdown_caret(true)
                            .disabled(disabled || key_count == 0)
                            .dropdown_menu(move |mut menu, _, _| {
                                for index in 0..key_count {
                                    let owner = key_owner.clone();
                                    menu = menu.item(PopupMenuItem::new(format!("密钥 {}", index + 1)).checked(index == active_key)
                                        .on_click(move |_, window, cx| {
                                            let _ = owner.update(cx, |this, cx| {
                                                this.capture_editor(window, cx);
                                                if let Some(provider) = this.draft.iter_mut().find(|provider| Some(&provider.id) == this.editing.as_ref()) {
                                                    provider.active_key_index = index;
                                                }
                                                this.dirty = true;
                                                cx.notify();
                                            });
                                        }));
                                }
                                menu
                            }))
                        .child(Button::new("provider-add-key").label("添加为新密钥").disabled(disabled)
                            .on_click(cx.listener(|this, _, window, cx| this.add_key(window, cx))))
                        .child(Button::new("provider-clear-key").label("移除当前密钥").disabled(disabled || key_count == 0)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.capture_editor(window, cx);
                                if let Some(provider) = this.draft.iter_mut().find(|provider| Some(&provider.id) == this.editing.as_ref()) {
                                    if !provider.api_keys.is_empty() { provider.api_keys.remove(provider.clamped_active_key_index()); }
                                    sanitize_provider(provider);
                                }
                                this.dirty = true;
                                cx.notify();
                            }))));
            }
            content = content
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_4()
                        .child(
                            Checkbox::new("provider-enabled")
                                .label("启用供应商")
                                .checked(provider.enabled)
                                .disabled(disabled)
                                .on_click(cx.listener(|this, value, _, cx| {
                                    if let Some(provider) = this.draft.iter_mut().find(|provider| {
                                        Some(&provider.id) == this.editing.as_ref()
                                    }) {
                                        provider.enabled = *value;
                                    }
                                    this.dirty = true;
                                    cx.notify();
                                })),
                        )
                        .child(
                            Checkbox::new("provider-proxy")
                                .label("跟随系统代理")
                                .checked(provider.request.use_system_proxy)
                                .disabled(disabled)
                                .on_click(cx.listener(|this, value, _, cx| {
                                    if let Some(provider) = this.draft.iter_mut().find(|provider| {
                                        Some(&provider.id) == this.editing.as_ref()
                                    }) {
                                        provider.request.use_system_proxy = *value;
                                    }
                                    this.dirty = true;
                                    cx.notify();
                                })),
                        ),
                )
                .child(
                    Button::new("provider-remove")
                        .label("移除此供应商")
                        .disabled(disabled || signed)
                        .tooltip(if signed {
                            "先断开账号再移除供应商"
                        } else {
                            "保存后移除此供应商"
                        })
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.draft.retain(|provider| provider.id != provider_id);
                            this.open_editor(
                                this.draft.first().map(|provider| provider.id.clone()),
                                window,
                                cx,
                            );
                            this.dirty = true;
                            cx.notify();
                        })),
                );
        } else {
            content = content.child(
                div()
                    .text_sm()
                    .child("添加你的模型供应商，填写 API 地址、密钥和模型后即可开始对话。"),
            );
        }
        content = content
            .when_some(self.error.clone(), |view, error| {
                view.child(div().text_sm().text_color(cx.theme().danger).child(error))
                    .child(
                        Button::new("providers-reload")
                            .label("放弃编辑并重新读取")
                            .disabled(self.busy)
                            .on_click(cx.listener(|this, _, window, cx| this.reload(window, cx))),
                    )
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        Button::new("providers-save")
                            .primary()
                            .label(if self.busy {
                                "正在处理…"
                            } else {
                                "保存配置"
                            })
                            .disabled(disabled || !self.dirty)
                            .on_click(cx.listener(|this, _, window, cx| this.save(window, cx))),
                    )
                    .child(
                        Button::new("providers-reset")
                            .label("放弃编辑")
                            .disabled(disabled || !self.dirty)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.accept(this.confirmed.clone(), window, cx);
                                cx.notify();
                            })),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(if self.dirty {
                                "有未保存的修改"
                            } else {
                                "已保存"
                            }),
                    ),
            );
        div()
            .v_flex()
            .size_full()
            .min_h_0()
            .text_size(px(14.))
            .child(
                div()
                    .flex_none()
                    .px_6()
                    .pt(px(48.))
                    .pb_5()
                    .child(
                        div()
                            .text_size(px(22.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("模型"),
                    )
                    .child(
                        div()
                            .mt_2()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child("管理 OpenAI 兼容供应商、密钥和启用模型。"),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .gap_5()
                    .px_6()
                    .pb_6()
                    .child(
                        div()
                            .id("provider-list-scroll")
                            .w(px(190.))
                            .flex_none()
                            .h_full()
                            .overflow_y_scroll()
                            .pr_4()
                            .border_r_1()
                            .border_color(cx.theme().border)
                            .child(providers),
                    )
                    .child(
                        div()
                            .id("provider-detail-scroll")
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .overflow_y_scroll()
                            .child(
                                div()
                                    .v_flex()
                                    .gap_4()
                                    .border_1()
                                    .border_color(cx.theme().border)
                                    .bg(cx.theme().background)
                                    .rounded_lg()
                                    .p_4()
                                    .child(
                                        div().font_weight(FontWeight::MEDIUM).child("供应商配置"),
                                    )
                                    .child(content),
                            ),
                    ),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::{ProviderDocument, ProviderSettings};
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{
        AppContext, Bounds, TestAppContext, WindowBounds, WindowOptions, point, px, size,
    };
    use serde_json::{Value, json};
    use std::fs;

    fn fixture() -> Value {
        json!({
            "providers": [{
                "id": "provider-one", "name": "Existing provider", "baseUrl": "https://example.test/v1",
                "apiKeys": ["test-credential-one", "test-credential-two"], "activeKeyIndex": 1,
                "availableModels": ["model-a", "model-b"], "enabledModels": ["model-a", "model-b"],
                "modelOverrides": {"model-a": {"temperature": 0.3, "extraBody": {"customFlag": true}}},
                "request": {"useSystemProxy": false, "customHeaders": [{"key": "x-workspace", "value": "test-workspace"}]}
            }],
            "defaultModels": {"chat": {"providerId": "provider-one", "model": "model-a"}, "translator": {"providerId": "translator", "model": "retained"}},
            "unrelatedSetting": {"preserve": true}
        })
    }

    // Real Kit input and Save → durable file → reopened owner. Failure preserves
    // both the confirmed runtime selection and the editable draft for retry.
    #[gpui_kit::test]
    fn provider_editor_preserves_credentials_and_retries_a_failed_save(cx: &mut TestAppContext) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("providers.json");
        let original = serde_json::to_vec(&fixture()).unwrap();
        fs::write(&path, &original).unwrap();
        let (window, settings) = cx.update(|cx| {
            gpui_kit::init(cx);
            cx.set_reduce_motion(true);
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                        point(px(0.), px(0.)),
                        size(px(1000.), px(1100.)),
                    ))),
                    ..Default::default()
                },
                cx,
                |window, cx| cx.new(|cx| ProviderSettings::new(path.clone(), window, cx)),
            )
            .unwrap()
        });
        cx.run_until_parked();
        assert_eq!(
            fs::read(&path).unwrap(),
            original,
            "opening settings must not rewrite them"
        );
        window
            .update(cx, |_, window, cx| {
                window.render_frame(cx);
                window.click("provider-name", cx);
            })
            .unwrap();
        cx.simulate_keystrokes(window, "secondary-a");
        cx.simulate_input(window, "Edited provider");
        window
            .update(cx, |_, window, cx| window.click("providers-save", cx))
            .unwrap();
        cx.run_until_parked();
        let saved: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(saved["providers"][0]["name"], "Edited provider");
        assert_eq!(
            saved["providers"][0]["apiKeys"],
            fixture()["providers"][0]["apiKeys"]
        );
        assert_eq!(saved["providers"][0]["activeKeyIndex"], 1);
        assert_eq!(
            saved["providers"][0]["modelOverrides"]["model-a"]["extraBody"],
            json!({"customFlag": true})
        );
        assert_eq!(
            saved["providers"][0]["request"]["customHeaders"],
            fixture()["providers"][0]["request"]["customHeaders"]
        );
        assert_eq!(
            saved["defaultModels"]["translator"],
            fixture()["defaultModels"]["translator"]
        );
        assert_eq!(saved["unrelatedSetting"], json!({"preserve": true}));

        window
            .update(cx, |_, window, cx| window.click("provider-key", cx))
            .unwrap();
        cx.simulate_input(window, "test-credential-three");
        window
            .update(cx, |_, window, cx| {
                window.click("provider-add-key", cx);
                window.click("providers-save", cx);
            })
            .unwrap();
        cx.run_until_parked();
        let saved: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(
            saved["providers"][0]["apiKeys"].as_array().unwrap().len(),
            3
        );
        assert_eq!(saved["providers"][0]["activeKeyIndex"], 2);
        window
            .update(cx, |_, window, cx| {
                window.click("provider-clear-key", cx);
                window.click("providers-save", cx);
            })
            .unwrap();
        cx.run_until_parked();
        let saved: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(
            saved["providers"][0]["apiKeys"],
            fixture()["providers"][0]["apiKeys"]
        );
        assert_eq!(saved["providers"][0]["activeKeyIndex"], 1);

        // The public selection intent is also used by the header. Changing the
        // default must leave a bound conversation's old model resolvable.
        window
            .update(cx, |_, window, cx| {
                settings.update(cx, |settings, cx| {
                    settings.select_model("provider-one".into(), "model-b".into(), window, cx);
                })
            })
            .unwrap();
        cx.run_until_parked();
        let (_, reopened) = cx.update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| ProviderSettings::new(path.clone(), window, cx))
            })
            .unwrap()
        });
        cx.run_until_parked();
        reopened.read_with(cx, |settings, _| {
            assert_eq!(settings.selection().unwrap().1, "model-b");
            assert_eq!(
                settings.selected("provider-one", "model-a").unwrap().1,
                "model-a"
            );
        });

        let committed = fs::read(&path).unwrap();
        window
            .update(cx, |_, window, cx| window.click("provider-name", cx))
            .unwrap();
        cx.simulate_keystrokes(window, "secondary-a");
        cx.simulate_input(window, "Retained draft");
        fs::remove_file(&path).unwrap();
        fs::create_dir(&path).unwrap();
        window
            .update(cx, |_, window, cx| window.click("providers-save", cx))
            .unwrap();
        cx.run_until_parked();
        settings.read_with(cx, |settings, cx| {
            assert!(settings.error.is_some());
            assert!(settings.dirty);
            assert_eq!(settings.name.read(cx).value(), "Retained draft");
            assert_eq!(settings.selection().unwrap().0.name, "Edited provider");
        });
        fs::remove_dir(&path).unwrap();
        fs::write(&path, committed).unwrap();
        window
            .update(cx, |_, window, cx| window.click("providers-save", cx))
            .unwrap();
        cx.run_until_parked();
        assert_eq!(
            ProviderDocument::load(&path).unwrap().providers[0].name,
            "Retained draft"
        );
    }

    #[test]
    fn concurrent_provider_writers_preserve_the_first_commit() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("providers.json");
        fs::write(&path, serde_json::to_vec(&fixture()).unwrap()).unwrap();
        let first = ProviderDocument::load(&path).unwrap();
        let second = first.clone();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let results = std::thread::scope(|scope| {
            let launch = |mut document: ProviderDocument, name: &'static str| {
                let barrier = barrier.clone();
                let path = path.clone();
                scope.spawn(move || {
                    document.providers[0].name = name.into();
                    barrier.wait();
                    document.save(&path).is_ok()
                })
            };
            let first = launch(first, "first writer");
            let second = launch(second, "second writer");
            [first.join().unwrap(), second.join().unwrap()]
        });
        assert_eq!(results.iter().filter(|result| **result).count(), 1);
        let name = ProviderDocument::load(&path).unwrap().providers[0]
            .name
            .clone();
        assert_eq!(
            name,
            if results[0] {
                "first writer"
            } else {
                "second writer"
            }
        );
    }
}
