//! The original search provider collection and its scoped configuration editor.
use gpui_kit::component::Sizable;
use gpui_kit::component::{
    ActiveTheme, Disableable, Selectable, StyledExt,
    button::{Button, ButtonVariants},
    input::{Input, InputEvent, InputState, Textarea, TextareaState},
    menu::{DropdownMenu, PopupMenuItem},
    switch::Switch,
};
use gpui_kit::{prelude::*, *};
use kivio_model_runtime::{
    native_preferences::{self, Change},
    settings_config::LensWebSearchConfig,
};
use serde_json::{Value, json};
use std::collections::HashMap;

// Presentation fields mirror WebSearchPanel.tsx; defaults and persistence are Rust-owned.
const PROVIDERS: [(&str, &str, &str, &str); 14] = [
    ("bocha", "Bocha", "bochaBaseUrl", "bochaApiKey"),
    ("brave", "Brave", "braveBaseUrl", "braveApiKey"),
    ("deepseek", "DeepSeek", "deepseekBaseUrl", "deepseekApiKey"),
    ("exa", "Exa", "exaBaseUrl", "exaApiKey"),
    ("exa_mcp", "ExaMCP", "exaMcpUrl", ""),
    ("grok", "Grok", "grokBaseUrl", "grokApiKey"),
    ("kimi", "Kimi", "kimiBaseUrl", "kimiApiKey"),
    ("ollama", "Ollama", "ollamaBaseUrl", "ollamaApiKey"),
    ("searxng", "SearXNG", "searxngBaseUrl", ""),
    ("serper", "Serper", "serperBaseUrl", "serperApiKey"),
    ("tavily", "Tavily", "tavilyBaseUrl", "tavilyApiKey"),
    ("tinyfish", "TinyFish", "tinyfishBaseUrl", "tinyfishApiKey"),
    ("tinyfish_mcp", "TinyFishMCP", "tinyfishMcpUrl", ""),
    ("zhipu", "Zhipu", "zhipuBaseUrl", "zhipuApiKey"),
];
struct SearchService {
    runtime: std::sync::Arc<kivio_model_runtime::web_search_host::NativeSearch>,
    _maintenance: Task<()>,
}
impl Global for SearchService {}
fn search_runtime(
    root: std::path::PathBuf,
    cx: &mut App,
) -> std::sync::Arc<kivio_model_runtime::web_search_host::NativeSearch> {
    if let Some(service) = cx.try_global::<SearchService>() {
        return service.runtime.clone();
    }
    let runtime = std::sync::Arc::new(kivio_model_runtime::web_search_host::NativeSearch::new(
        root,
    ));
    let maintenance = runtime.clone();
    let task = cx.spawn(async move |cx| {
        loop {
            cx.background_executor()
                .timer(std::time::Duration::from_secs(30))
                .await;
            let runtime = maintenance.clone();
            cx.background_spawn(async move {
                if let Ok(executor) = crate::chat_runtime::runtime() {
                    executor.block_on(runtime.maintain());
                }
            })
            .await;
        }
    });
    let shutdown = runtime.clone();
    cx.on_app_quit(move |cx| {
        let runtime = shutdown.clone();
        let work = cx.background_spawn(async move {
            if let Ok(executor) = crate::chat_runtime::runtime() {
                executor.block_on(runtime.shutdown());
            }
        });
        async move {
            work.await;
        }
    })
    .detach();
    cx.set_global(SearchService {
        runtime: runtime.clone(),
        _maintenance: task,
    });
    runtime
}
pub struct SearchPage {
    english_placeholders: bool,
    directory: std::path::PathBuf,
    selected: Option<usize>,
    confirmed: Option<LensWebSearchConfig>,
    draft: Value,
    fields: HashMap<&'static str, Entity<InputState>>,
    prompt: Entity<TextareaState>,
    busy: bool,
    error: Option<String>,
    saved: bool,
    task: Option<Task<()>>,
    runtime: std::sync::Arc<kivio_model_runtime::web_search_host::NativeSearch>,
    query: Entity<InputState>,
    output: Entity<TextareaState>,
    probing: bool,
    probe_task: Option<Task<()>>,
    browser_task: Option<Task<()>>,
    cancel: Option<tokio::sync::oneshot::Sender<()>>,
    _editors: Vec<Subscription>,
}
impl SearchPage {
    pub fn new(directory: std::path::PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut fields = HashMap::new();
        for (_, _, endpoint, key) in PROVIDERS {
            fields.insert(endpoint, cx.new(|cx| InputState::new(window, cx)));
            if !key.is_empty() {
                fields.insert(key, cx.new(|cx| InputState::new(window, cx).masked(true)));
            }
        }
        fields.insert("grokModel", cx.new(|cx| InputState::new(window, cx)));
        let prompt = cx.new(|cx| TextareaState::new(window, cx).rows(5));
        let runtime = search_runtime(directory.clone(), cx);
        let query =
            cx.new(|cx| InputState::new(window, cx).placeholder(crate::i18n::text("输入搜索词，或粘贴网页 URL")));
        let output = cx.new(|cx| TextareaState::new(window, cx).rows(8));
        let mut this = Self {
            english_placeholders: crate::i18n::is_english(),
            runtime,
            query,
            output,
            probing: false,
            probe_task: None,
            browser_task: None,
            cancel: None,
            _editors: vec![],
            directory,
            selected: None,
            confirmed: None,
            draft: json!({}),
            fields,
            prompt,
            busy: false,
            error: None,
            saved: false,
            task: None,
        };
        this._editors = this
            .fields
            .values()
            .map(|input| {
                cx.subscribe(input, |this, _, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Change) {
                        this.saved = false;
                        cx.notify();
                    }
                })
            })
            .collect();
        this._editors.push(
            cx.subscribe(&this.prompt, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.saved = false;
                    cx.notify();
                }
            }),
        );
        this.load(false, window, cx);
        this
    }
    pub(crate) fn is_busy(&self) -> bool {
        self.busy || self.probing
    }
    fn select_provider(
        &mut self,
        selected: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.is_busy() || self.selected == selected {
            return;
        }
        self.selected = selected;
        self.output
            .update(cx, |output, cx| output.set_value("", window, cx));
        self.error = None;
        cx.notify();
    }
    fn current_draft(&self, cx: &App) -> Value {
        let mut draft = self.draft.clone();
        for (key, input) in &self.fields {
            draft[*key] = json!(input.read(cx).value().to_string());
        }
        draft["grokSystemPrompt"] = json!(self.prompt.read(cx).value().to_string());
        draft
    }
    pub(crate) fn has_unsaved_changes(&self, cx: &App) -> bool {
        match &self.confirmed {
            Some(config) => {
                self.current_draft(cx)
                    != serde_json::to_value(config).expect("serializable search config")
            }
            None => {
                self.fields
                    .values()
                    .any(|input| !input.read(cx).value().is_empty())
                    || !self.prompt.read(cx).value().is_empty()
            }
        }
    }
    fn load(&mut self, save: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || self.probing {
            return;
        }
        let change = if save {
            let Some(before) = self.confirmed.clone() else {
                return;
            };
            let draft = self.current_draft(cx);
            let after: LensWebSearchConfig = match serde_json::from_value(draft) {
                Ok(value) => value,
                Err(error) => {
                    self.error = Some(error.to_string());
                    cx.notify();
                    return;
                }
            };
            Some(Change::WebSearch { before, after })
        } else {
            None
        };
        self.busy = true;
        self.error = None;
        self.saved = false;
        let directory = self.directory.clone();
        let handle = window.window_handle();
        self.task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    match change {
                        Some(change) => native_preferences::save(&directory, change),
                        None => native_preferences::load(&directory),
                    }
                })
                .await;
            let _ = cx.update_window(handle, |_, window, cx| {
                this.update(cx, |this, cx| {
                    this.busy = false;
                    match result {
                        Ok(settings) => {
                            let config = settings.lens.web_search;
                            this.draft =
                                serde_json::to_value(&config).expect("serializable search config");
                            for (key, input) in &this.fields {
                                let value = this.draft[*key].as_str().unwrap_or("").to_owned();
                                input.update(cx, |input, cx| input.set_value(value, window, cx));
                            }
                            this.prompt.update(cx, |input, cx| {
                                input.set_value(config.grok_system_prompt.clone(), window, cx)
                            });
                            this.confirmed = Some(config);
                            this.saved = save;
                        }
                        Err(error) => this.error = Some(error),
                    }
                    cx.notify();
                })
            });
        }));
        cx.notify();
    }
    fn authorize(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || self.probing {
            return;
        }
        let Some(before) = self.confirmed.clone() else {
            return;
        };
        let endpoint = self.fields["tinyfishMcpUrl"]
            .read(cx)
            .value()
            .trim()
            .to_string();
        if !endpoint.starts_with("https://") {
            self.error = Some("OAuth 地址必须使用 HTTPS".into());
            cx.notify();
            return;
        }
        let root = self.directory.clone();
        let runtime = self.runtime.clone();
        let (urls, incoming) = async_channel::unbounded::<String>();
        self.browser_task = Some(cx.spawn(async move |_, cx| {
            while let Ok(url) = incoming.recv().await {
                let _ = cx.update(|cx| cx.open_url(&url));
            }
        }));
        let (cancel, cancelled) = tokio::sync::oneshot::channel();
        self.cancel = Some(cancel);
        self.busy = true;
        self.probing = true;
        self.error = None;
        self.saved = false;
        self.probe_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    crate::chat_runtime::runtime()?.block_on(async move {
                        let browser = move |url: &str| {
                            urls.try_send(url.to_string())
                                .map_err(|_| "授权窗口已关闭".to_string())
                        };
                        let server = tokio::select! {
                            _ = cancelled => return Err("已取消授权".to_string()),
                            result = runtime.authorize(&browser, &endpoint) => result?,
                        };
                        let mut after = before.clone();
                        after.tinyfish_mcp_url = endpoint;
                        after.tinyfish_mcp_auth = server.auth;
                        native_preferences::save(&root, Change::WebSearch { before, after })
                    })
                })
                .await;
            let _ = this.update_in(cx, |this, _, cx| {
                this.busy = false;
                this.probing = false;
                this.cancel = None;
                this.browser_task = None;
                match result {
                    Ok(settings) => {
                        this.draft["tinyfishMcpAuth"] =
                            json!(settings.lens.web_search.tinyfish_mcp_auth);
                        this.draft["tinyfishMcpUrl"] =
                            json!(settings.lens.web_search.tinyfish_mcp_url);
                        this.confirmed = Some(settings.lens.web_search);
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }
    fn probe(&mut self, fetch: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.probing || self.busy || self.confirmed.is_none() {
            return;
        }
        let query = self.query.read(cx).value().trim().to_string();
        if query.is_empty() {
            self.error = Some("请输入搜索词或网页 URL".into());
            cx.notify();
            return;
        }
        let mut draft = self.current_draft(cx);
        if let Some(index) = self.selected {
            draft["provider"] = json!(PROVIDERS[index].0);
            if fetch {
                draft["fetchProvider"] = draft["provider"].clone();
            }
        }
        let config: LensWebSearchConfig = match serde_json::from_value(draft) {
            Ok(value) => value,
            Err(error) => {
                self.error = Some(error.to_string());
                cx.notify();
                return;
            }
        };
        let runtime = self.runtime.clone();
        let (cancel, cancelled) = tokio::sync::oneshot::channel();
        self.cancel = Some(cancel);
        self.probing = true;
        self.error = None;
        self.output
            .update(cx, |output, cx| output.set_value("", window, cx));
        self.probe_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = cx.background_spawn(async move {
                crate::chat_runtime::runtime()?.block_on(async move {
                    tokio::select! {
                        _ = cancelled => Err("已取消测试".to_string()),
                        result = async {
                            if fetch { runtime.fetch(&config, &query).await.map(|page| kivio_model_runtime::web_search::format_web_fetch(&page)) }
                            else { runtime.search(&config, &query).await.map(|results| if results.is_empty() { "请求成功，未返回结果".into() } else { kivio_model_runtime::web_search::format_web_context(&results) }) }
                        } => result,
                    }
                })
            }).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.probing = false; this.cancel = None;
                match result { Ok(result) => this.output.update(cx, |output, cx| output.set_value(result, window, cx)), Err(error) => this.error = Some(error) }
                cx.notify();
            });
        }));
        cx.notify();
    }
    fn choice(
        &self,
        field: &'static str,
        options: Vec<(String, String)>,
        cx: &Context<Self>,
    ) -> AnyElement {
        let current = if self.draft[field].is_null() {
            String::new()
        } else if let Some(text) = self.draft[field].as_str() {
            text.into()
        } else {
            self.draft[field].to_string()
        };
        let label = options
            .iter()
            .find(|(key, _)| *key == current)
            .map(|(_, label)| label.clone())
            .unwrap_or(current.clone());
        let owner = cx.weak_entity();
        Button::new(field)
            .small()
            .label(crate::i18n::display(label))
            .dropdown_caret(true)
            .disabled(self.busy || self.confirmed.is_none())
            .dropdown_menu(move |mut menu, _, _| {
                for (value, label) in &options {
                    let owner = owner.clone();
                    let value = value.clone();
                    menu = menu.item(
                        PopupMenuItem::new(label.clone())
                            .checked(value == current)
                            .on_click(move |_, _, cx| {
                                let _ = owner.update(cx, |this, cx| {
                                    this.draft[field] = if field == "maxResults" {
                                        json!(value.parse::<u8>().unwrap_or(5))
                                    } else if field == "fetchProvider" && value.is_empty() {
                                        Value::Null
                                    } else {
                                        json!(value)
                                    };
                                    this.saved = false;
                                    cx.notify();
                                });
                            }),
                    );
                }
                menu
            })
            .into_any_element()
    }
}
impl Render for SearchPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let english = crate::i18n::is_english();
        if self.english_placeholders != english {
            self.english_placeholders = english;
            self.query.update(cx, |input, cx| input.set_placeholder(crate::i18n::text("输入搜索词，或粘贴网页 URL"), window, cx));
        }
        let mut nav = div()
            .id("search-provider-list")
            .flex()
            .flex_wrap()
            .gap_1()
            .p_1()
            .rounded(px(10.))
            .bg(cx.theme().group_box)
            .child(
                Button::new("search-defaults")
                    .small()
                    .ghost()
                    .label(crate::i18n::text("默认设置"))
                    .selected(self.selected.is_none())
                    .disabled(self.is_busy())
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.select_provider(None, window, cx);
                    })),
            );
        for (index, (_, label, _, _)) in PROVIDERS.iter().enumerate() {
            nav = nav.child(
                Button::new(("search-provider", index))
                    .small()
                    .ghost()
                    .label(*label)
                    .selected(self.selected == Some(index))
                    .disabled(self.is_busy())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.select_provider(Some(index), window, cx);
                    })),
            );
        }
        let row = |label: &'static str, control: AnyElement| {
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap_4()
                .py_3()
                .border_b_1()
                .border_color(cx.theme().border)
                .child(crate::i18n::display(label))
                .child(control)
        };
        let mut details = div().v_flex().gap_3().min_w_0();
        if let Some(index) = self.selected {
            let (id, label, endpoint, key) = PROVIDERS[index];
            details = details
                .child(
                    div()
                        .text_size(px(14.))
                        .font_weight(FontWeight::MEDIUM)
                        .child(crate::i18n::display(label)),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(crate::i18n::text("配置服务商的接口地址与凭据。")),
                )
                .child(
                    div()
                        .v_flex()
                        .gap_2()
                        .child(if endpoint.ends_with("McpUrl") { crate::i18n::text("MCP 地址") } else { crate::i18n::text("API 地址") })
                        .child(
                            Input::new(&self.fields[endpoint])
                                .aria_label("服务商接口地址")
                                .small()
                                .disabled(self.busy),
                        ),
                );
            if !key.is_empty() {
                details = details.child(
                    div().v_flex().gap_2().child("API Key").child(
                        Input::new(&self.fields[key])
                            .aria_label("服务商 API Key")
                            .small()
                            .disabled(self.busy),
                    ),
                );
            }
            if id == "grok" {
                details = details
                    .child(
                        div().v_flex().gap_2().child(crate::i18n::text("搜索模型")).child(
                            Input::new(&self.fields["grokModel"])
                                .aria_label("Grok 搜索模型")
                                .small()
                                .disabled(self.busy),
                        ),
                    )
                    .child(div().child(crate::i18n::text("系统提示词")))
                    .child(
                        Textarea::new(&self.prompt)
                            .aria_label("搜索系统提示词")
                            .disabled(self.busy),
                    );
            }
            if id == "tavily" {
                details = details.child(row(
                    "搜索深度",
                    self.choice(
                        "searchDepth",
                        ["ultra-fast", "fast", "basic", "advanced"]
                            .into_iter()
                            .map(|s| (s.into(), s.into()))
                            .collect(),
                        cx,
                    ),
                ));
            }
            if id == "exa_mcp" {
                details = details.child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(crate::i18n::text("ExaMCP 使用 MCP 地址，无需 API Key。")),
                );
            }
            if id == "searxng" {
                details = details.child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(crate::i18n::text("填写支持 JSON 搜索结果的 SearXNG 实例地址。")),
                );
            }
            if id == "tinyfish_mcp" {
                details = details.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(if self.draft["tinyfishMcpAuth"].is_object() { crate::i18n::text("已授权") } else { crate::i18n::text("尚未授权") })
                        .child(
                            Button::new("tinyfish-authorize")
                                .small()
                                .label(crate::i18n::text("浏览器授权…"))
                                .disabled(self.busy || self.probing)
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.authorize(window, cx)),
                                ),
                        )
                        .when(self.draft["tinyfishMcpAuth"].is_object(), |row| {
                            row.child(
                                Button::new("tinyfish-disconnect")
                                    .small()
                                    .label(crate::i18n::text("移除授权"))
                                    .disabled(self.busy || self.probing)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.draft["tinyfishMcpAuth"] = Value::Null;
                                        this.saved = false;
                                        cx.notify();
                                    })),
                            )
                        }),
                );
                details = details.child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(crate::i18n::text("TinyFish MCP 使用浏览器授权；令牌会自动刷新。")),
                );
            }
        } else {
            let options = PROVIDERS
                .iter()
                .map(|(id, name, _, _)| (id.to_string(), name.to_string()))
                .collect();
            let mut fetch = vec![(String::new(), "跟随搜索服务商".into())];
            fetch.extend(
                PROVIDERS
                    .iter()
                    .filter(|(id, _, _, _)| {
                        matches!(
                            *id,
                            "tavily"
                                | "exa"
                                | "exa_mcp"
                                | "ollama"
                                | "deepseek"
                                | "serper"
                                | "tinyfish"
                                | "tinyfish_mcp"
                                | "kimi"
                        )
                    })
                    .map(|(id, name, _, _)| (id.to_string(), name.to_string())),
            );
            details = details
                .child(
                    div()
                        .text_size(px(14.))
                        .font_weight(FontWeight::MEDIUM)
                        .child(crate::i18n::text("默认设置")),
                )
                .child(row("搜索服务商", self.choice("provider", options, cx)))
                .child(row(
                    "网页抓取服务商",
                    self.choice("fetchProvider", fetch, cx),
                ))
                .child(row(
                    "结果数量",
                    self.choice(
                        "maxResults",
                        (1..=10).map(|n| (n.to_string(), n.to_string())).collect(),
                        cx,
                    ),
                ))
                .child(row(
                    "Lens 联网搜索",
                    Switch::new("lens-search-enabled")
                        .accessibility_label(crate::i18n::text("Lens 联网搜索"))
                        .checked(self.draft["enabled"].as_bool().unwrap_or(false))
                        .disabled(self.busy || self.confirmed.is_none())
                        .on_click(cx.listener(|this, value: &bool, _, cx| {
                            this.draft["enabled"] = json!(*value);
                            this.saved = false;
                            cx.notify();
                        }))
                        .into_any_element(),
                ));
        }
        details =
            details.child(
                div()
                    .v_flex()
                    .gap_2()
                    .pt_4()
                    .child(crate::i18n::text("服务测试"))
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(crate::i18n::text("使用当前页面的配置发送实际请求；服务商可能计费。")),
                    )
                    .child(
                        Input::new(&self.query)
                            .small()
                            .aria_label("搜索测试内容")
                            .disabled(self.probing),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(
                                Button::new("search-test")
                                    .small()
                                    .label(crate::i18n::text("搜索"))
                                    .disabled(self.probing || self.busy || self.confirmed.is_none())
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.probe(false, window, cx)
                                    })),
                            )
                            .child(
                                Button::new("fetch-test")
                                    .small()
                                    .label(crate::i18n::text("抓取网页"))
                                    .disabled(self.probing || self.busy || self.confirmed.is_none())
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.probe(true, window, cx)
                                    })),
                            )
                            .when(self.probing, |row| {
                                row.child(
                                    Button::new("search-test-stop")
                                        .small()
                                        .label(crate::i18n::text("停止"))
                                        .on_click(cx.listener(|this, _, _, _| {
                                            if let Some(cancel) = this.cancel.take() {
                                                let _ = cancel.send(());
                                            }
                                        })),
                                )
                            }),
                    )
                    .when(!self.output.read(cx).value().is_empty(), |view| {
                        view.child(Textarea::new(&self.output).aria_label("搜索测试结果"))
                    }),
            );
        if let Some(error) = &self.error {
            details = details.child(div().text_color(cx.theme().danger).child(error.clone()));
        }
        let dirty = self.has_unsaved_changes(cx);
        details = details.child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .justify_end()
                .gap_2()
                .when(dirty, |row| {
                    row.child(div().text_sm().child(crate::i18n::text("有未保存的修改")))
                })
                .when(self.saved && !dirty, |row| {
                    row.child(div().text_sm().child(crate::i18n::text("已保存")))
                })
                .child(
                    Button::new("search-reload")
                        .small()
                        .label(if dirty { crate::i18n::text("放弃修改并重新读取") } else { crate::i18n::text("重新读取") })
                        .disabled(self.busy || self.probing)
                        .on_click(cx.listener(|this, _, window, cx| this.load(false, window, cx))),
                )
                .child(
                    Button::new("search-save")
                        .small()
                        .primary()
                        .label(if self.busy { crate::i18n::text("读取中…") } else { crate::i18n::text("保存设置") })
                        .disabled(self.busy || self.probing || self.confirmed.is_none() || !dirty)
                        .on_click(cx.listener(|this, _, window, cx| this.load(true, window, cx))),
                ),
        );
        let content = div().v_flex().gap_5().child(nav).child(
            div()
                .flex_1()
                .min_w_0()
                .bg(cx.theme().group_box)
                .rounded(px(12.))
                .p_4()
                .child(details),
        );
        crate::kivio_settings::page_frame(
            "网络搜索",
            "选择搜索和网页抓取服务商，并管理各服务商配置。",
            content.into_any_element(),
            cx,
        )
    }
}
