//! The original search provider collection and its scoped configuration editor.
use gpui_kit::component::{
    ActiveTheme, Disableable, Selectable, StyledExt,
    button::{Button, ButtonVariants},
    input::{Input, InputState, Textarea, TextareaState},
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
pub struct SearchPage {
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
        let mut this = Self {
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
        this.load(false, window, cx);
        this
    }
    fn load(&mut self, save: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let change = if save {
            let Some(before) = self.confirmed.clone() else {
                return;
            };
            let mut draft = self.draft.clone();
            for (key, input) in &self.fields {
                draft[*key] = json!(input.read(cx).value().to_string());
            }
            draft["grokSystemPrompt"] = json!(self.prompt.read(cx).value().to_string());
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
            .label(label)
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
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut nav = div()
            .id("search-provider-list")
            .w(px(156.))
            .flex_shrink_0()
            .v_flex()
            .gap_1()
            .overflow_y_scroll()
            .child(
                Button::new("search-defaults")
                    .ghost()
                    .w_full()
                    .justify_start()
                    .label("默认设置")
                    .selected(self.selected.is_none())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.selected = None;
                        cx.notify();
                    })),
            );
        for (index, (_, label, _, _)) in PROVIDERS.iter().enumerate() {
            nav = nav.child(
                Button::new(("search-provider", index))
                    .ghost()
                    .w_full()
                    .justify_start()
                    .label(*label)
                    .selected(self.selected == Some(index))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.selected = Some(index);
                        cx.notify();
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
                .child(label)
                .child(control)
        };
        let mut details = div().v_flex().gap_3().min_w_0();
        if let Some(index) = self.selected {
            let (id, label, endpoint, key) = PROVIDERS[index];
            details = details
                .child(
                    div()
                        .text_lg()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(label),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child("配置服务商的接口地址与凭据。"),
                )
                .child(
                    div()
                        .v_flex()
                        .gap_2()
                        .child(if endpoint.ends_with("McpUrl") {
                            "MCP 地址"
                        } else {
                            "API 地址"
                        })
                        .child(Input::new(&self.fields[endpoint]).disabled(self.busy)),
                );
            if !key.is_empty() {
                details = details.child(
                    div()
                        .v_flex()
                        .gap_2()
                        .child("API Key")
                        .child(Input::new(&self.fields[key]).disabled(self.busy)),
                );
            }
            if id == "grok" {
                details = details
                    .child(
                        div()
                            .v_flex()
                            .gap_2()
                            .child("搜索模型")
                            .child(Input::new(&self.fields["grokModel"]).disabled(self.busy)),
                    )
                    .child(div().child("系统提示词"))
                    .child(Textarea::new(&self.prompt).disabled(self.busy));
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
                        .child("ExaMCP 使用 MCP 地址，无需 API Key。"),
                );
            }
            if id == "searxng" {
                details = details.child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child("填写支持 JSON 搜索结果的 SearXNG 实例地址。"),
                );
            }
            if id == "tinyfish_mcp" {
                details = details.child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(
                            "TinyFishMCP 使用 OAuth。已保存的授权会保留；原生授权入口尚未接入。",
                        ),
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
                        .text_lg()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("默认设置"),
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
        if let Some(error) = &self.error {
            details = details.child(div().text_color(cx.theme().danger).child(error.clone()));
        }
        details = details.child(
            div()
                .flex()
                .items_center()
                .justify_end()
                .gap_2()
                .when(self.saved, |row| row.child(div().text_sm().child("已保存")))
                .child(
                    Button::new("search-save")
                        .primary()
                        .label(if self.busy {
                            "读取中…"
                        } else {
                            "保存设置"
                        })
                        .disabled(self.busy || self.confirmed.is_none())
                        .on_click(cx.listener(|this, _, window, cx| this.load(true, window, cx))),
                ),
        );
        let content = div().flex().items_start().gap_5().child(nav).child(
            div()
                .flex_1()
                .min_w_0()
                .border_1()
                .border_color(cx.theme().border)
                .rounded_lg()
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
