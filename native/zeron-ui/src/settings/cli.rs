//! CLI configuration editor. Native account/protocol execution remains owned by
//! the external-agent adapter and is never replaced by an ordinary model call.
use gpui_kit::component::Sizable;
use gpui_kit::component::{
    ActiveTheme, Disableable, Selectable, StyledExt,
    button::{Button, ButtonVariants},
    input::{Input, InputState, Textarea, TextareaState},
    switch::Switch,
};
use gpui_kit::{prelude::*, *};
use kivio_model_runtime::{
    native_preferences::{self, Change},
    settings_config::{CliCustomModel, CliEnvVar, ExternalCliAgentConfig},
};
use std::collections::HashMap;

const CLIS: &[(&str, &str)] = &[
    ("claude", "Claude Code"),
    ("codex", "Codex"),
    ("opencode", "OpenCode"),
    ("gemini", "Gemini CLI"),
    ("kimi", "Kimi CLI"),
    ("pi", "Pi"),
    ("hermes", "Hermes"),
    ("grok", "Grok"),
    ("dsh", "DeepSeek"),
    ("antigravity", "Antigravity"),
];
#[derive(Default)]
struct InstallProgress {
    lines: std::collections::VecDeque<SharedString>,
    result: Option<Result<(), String>>,
}
pub struct CliPage {
    english_placeholders: bool,
    directory: std::path::PathBuf,
    selected: String,
    configs: HashMap<String, ExternalCliAgentConfig>,
    discovery: HashMap<String, Result<kivio_model_runtime::external_agents::management::InstalledCli, String>>,
    discovery_tasks: HashMap<String, Task<()>>,
    discovery_generation: u64,
    install_jobs: HashMap<String, Task<()>>,
    install_progress: HashMap<String, InstallProgress>,
    install_info: Option<kivio_model_runtime::external_agents::installer::InstallInfo>,
    info_task: Option<Task<()>>,
    info_busy: bool,
    info_error: Option<String>,
    account: Option<serde_json::Value>,
    account_open: bool,
    account_error: Option<String>,
    account_task: Option<Task<()>>,
    account_busy: bool,
    path: Entity<InputState>,
    env: Entity<TextareaState>,
    models: Entity<TextareaState>,
    disabled: bool,
    editor_baseline: (bool, SharedString, SharedString, SharedString),
    busy: bool,
    ready: bool,
    error: Option<String>,
    task: Option<Task<()>>,
    subscriptions: Vec<Subscription>,
}
impl CliPage {
    pub(crate) fn is_busy(&self) -> bool {
        self.busy || !self.install_jobs.is_empty()
    }

    pub(crate) fn has_unsaved_changes(&self, cx: &App) -> bool {
        self.disabled != self.editor_baseline.0
            || self.path.read(cx).value() != self.editor_baseline.1
            || self.env.read(cx).value() != self.editor_baseline.2
            || self.models.read(cx).value() != self.editor_baseline.3
    }

    pub fn new(directory: std::path::PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            english_placeholders: crate::i18n::is_english(),
            directory,
            selected: "claude".into(),
            configs: HashMap::new(),
            discovery: HashMap::new(), discovery_tasks: HashMap::new(), discovery_generation: 0,
            install_jobs: HashMap::new(), install_progress: HashMap::new(),
            install_info: None, info_task: None, info_busy: false, info_error: None,
            account: None, account_open: false, account_error: None, account_task: None, account_busy: false,
            path: cx.new(|cx| InputState::new(window, cx).placeholder(crate::i18n::text("自动检测 PATH 中的命令"))),
            env: cx.new(|cx| TextareaState::new(window, cx).rows(5)),
            models: cx.new(|cx| TextareaState::new(window, cx).rows(4)),
            disabled: false,
            editor_baseline: (false, "".into(), "".into(), "".into()),
            busy: false,
            ready: false,
            error: None,
            task: None,
            subscriptions: vec![],
        };
        this.subscriptions = vec![
            cx.observe(&this.path, |_, _, cx| cx.notify()),
            cx.observe(&this.env, |_, _, cx| cx.notify()),
            cx.observe(&this.models, |_, _, cx| cx.notify()),
        ];
        this.load(false, window, cx);
        this
    }
    fn select(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        self.info_task = None;
        self.install_info = None;
        self.info_busy = false;
        self.info_error = None;
        self.account_task = None;
        self.account = None;
        self.account_error = None;
        self.account_busy = false;
        self.account_open = false;
        self.selected = id;
        let config = self
            .configs
            .get(&self.selected)
            .cloned()
            .unwrap_or_default();
        self.disabled = config.disabled;
        self.path
            .update(cx, |input, cx| input.set_value(config.path, window, cx));
        self.env.update(cx, |input, cx| {
            input.set_value(
                config
                    .env
                    .iter()
                    .map(|entry| format!("{}={}", entry.key, entry.value))
                    .collect::<Vec<_>>()
                    .join("\n"),
                window,
                cx,
            )
        });
        self.models.update(cx, |input, cx| {
            input.set_value(
                config
                    .custom_models
                    .iter()
                    .map(|model| {
                        if model.label.is_empty() {
                            model.id.clone()
                        } else {
                            format!("{}={}", model.id, model.label)
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n"),
                window,
                cx,
            )
        });
        self.editor_baseline = (
            self.disabled,
            self.path.read(cx).value(),
            self.env.read(cx).value(),
            self.models.read(cx).value(),
        );
        cx.notify();
    }
    fn detect(&mut self, id: String, cx: &mut Context<Self>) {
        let root = self.directory.clone();
        let key = id.clone();
        let task_key = id.clone();
        let generation = self.discovery_generation;
        let task = cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(async move {
                crate::chat_runtime::runtime()?.block_on(kivio_model_runtime::external_agents::management::inspect(&root, &id))
            }).await;
            let _ = this.update(cx, |this, cx| {
                if generation != this.discovery_generation { return; }
                this.discovery.insert(key.clone(), result);
                this.discovery_tasks.remove(&key);
                cx.notify();
            });
        });
        self.discovery_tasks.insert(task_key, task);
        cx.notify();
    }
    fn install(&mut self, cx: &mut Context<Self>) {
        if !self.install_jobs.is_empty() || self.busy || self.has_unsaved_changes(cx) { return; }
        let id = self.selected.clone();
        let key = id.clone();
        let root = self.directory.clone();
        self.install_progress.insert(id.clone(), InstallProgress::default());
        let task = cx.spawn(async move |this, cx| {
            let (sender, receiver) = async_channel::bounded::<String>(128);
            let work = cx.background_spawn(async move {
                crate::chat_runtime::runtime()?.block_on(kivio_model_runtime::external_agents::management::install(&root, &id, &|line| { let _ = sender.try_send(line); }))
            });
            while let Ok(line) = receiver.recv().await {
                let mut lines = vec![line];
                while let Ok(line) = receiver.try_recv() { lines.push(line); }
                if this.update(cx, |this, cx| {
                    let progress = this.install_progress.entry(key.clone()).or_default();
                    for line in lines { progress.lines.push_back(line.chars().take(2000).collect::<String>().into()); }
                    while progress.lines.len() > 200 { progress.lines.pop_front(); }
                    cx.notify();
                }).is_err() { return; }
            }
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                this.install_jobs.remove(&key);
                let progress = this.install_progress.entry(key.clone()).or_default();
                match result {
                    Ok(cli) => {
                        this.discovery.insert(key.clone(), Ok(cli));
                        progress.result = Some(Ok(()));
                    }
                    Err(error) => progress.result = Some(Err(error)),
                }
                if this.selected == key { this.install_info = None; }
                cx.notify();
            });
        });
        self.install_jobs.insert(self.selected.clone(), task);
        cx.notify();
    }
    fn refresh_info(&mut self, cx: &mut Context<Self>) {
        if self.info_busy || self.has_unsaved_changes(cx) { return; }
        self.info_busy = true;
        self.info_error = None;
        let root = self.directory.clone();
        let id = self.selected.clone();
        self.info_task = Some(cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(async move {
                crate::chat_runtime::runtime()?.block_on(kivio_model_runtime::external_agents::management::info(&root, &id))
            }).await;
            let _ = this.update(cx, |this, cx| {
                this.info_busy = false;
                match result { Ok(value) => this.install_info = Some(value), Err(error) => this.info_error = Some(error) }
                cx.notify();
            });
        }));
        cx.notify();
    }
    fn refresh_account(&mut self, cx: &mut Context<Self>) {
        if self.account_busy || self.has_unsaved_changes(cx) { return; }
        self.account_open = true;
        self.account_busy = true;
        self.account_error = None;
        let root = self.directory.clone();
        let id = self.selected.clone();
        self.account_task = Some(cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(async move {
                crate::chat_runtime::runtime()?.block_on(kivio_model_runtime::external_agents::management::account(&root, &id))
            }).await;
            let _ = this.update(cx, |this, cx| {
                this.account_busy = false;
                match result { Ok(value) => this.account = Some(value), Err(error) => this.account_error = Some(error) }
                cx.notify();
            });
        }));
        cx.notify();
    }
    fn management_controls(&self, cx: &mut Context<Self>) -> Div {
        let mut content = div().v_flex().gap_3();
        let id = self.selected.clone();
        let scanning = self.discovery_tasks.contains_key(&id);
        let disabled = self.busy || self.has_unsaved_changes(cx);
        let status = match self.discovery.get(&id) {
            Some(Ok(cli)) if cli.path.is_some() => format!("{} · {}", crate::i18n::text(if self.disabled { "已停用" } else { "已安装" }), cli.version.as_deref().unwrap_or("—")),
            Some(Ok(_)) => crate::i18n::text("未安装").into(),
            Some(Err(error)) => error.clone(),
            None => crate::i18n::text("正在检测…").into(),
        };
        let mut actions = div().flex().flex_wrap().gap_2()
            .child(Button::new("cli-detect").small().label(crate::i18n::text("重新检测")).disabled(disabled || scanning)
                .on_click(cx.listener(move |this, _, _, cx| { kivio_model_runtime::external_agents::spawn::clear_probe_cache(); this.detect(id.clone(), cx); })));
        if let Some(def) = kivio_model_runtime::external_agents::management_catalog::get(&self.selected) {
            let url = def.install.docs;
            actions = actions.child(Button::new("cli-docs").small().label(crate::i18n::text("官方文档"))
                .on_click(move |_, _, cx| cx.open_url(url)));
        }
        if matches!(self.selected.as_str(), "codex" | "claude" | "grok" | "antigravity") {
            actions = actions.child(Button::new("cli-account").small().label(crate::i18n::text(if self.account_open { "刷新账号与额度" } else { "CLI 账号与额度" }))
                .disabled(disabled || self.account_busy).on_click(cx.listener(|this, _, _, cx| this.refresh_account(cx))));
        }
        actions = actions.child(Button::new("cli-version-info").small().label(crate::i18n::text(if self.info_busy { "检查中…" } else { "检查更新" }))
            .disabled(disabled || self.info_busy).on_click(cx.listener(|this, _, _, cx| this.refresh_info(cx))));
        content = content.child(status).child(actions);
        if let Some(info) = &self.install_info {
            if info.command.is_some() && self.selected != "dsh" {
                content = content.child(Button::new("cli-install").small().primary()
                    .label(crate::i18n::text(if info.local_version.is_some() { "更新 CLI" } else { "安装 CLI" }))
                    .disabled(disabled || !self.install_jobs.is_empty())
                    .on_click(cx.listener(|this, _, _, cx| this.install(cx))));
            }
            content = content.child(crate::ui_format!("最新版本：{}", "Latest version: {}", info.latest_version.as_deref().unwrap_or("—")));
            if info.latest_version.is_none() { content = content.child(crate::i18n::text("暂时无法获取最新版本，请稍后重试。")); }
            if info.update_available { content = content.child(crate::i18n::text("有可用更新")); }
            if let Some(directory) = &info.config_dir {
                let path = std::path::PathBuf::from(directory);
                content = content.child(Button::new("cli-config-directory").small().label(crate::i18n::text("打开配置目录"))
                    .on_click(move |_, _, cx| cx.reveal_path(&path)));
            }
        }
        if let Some(progress) = self.install_progress.get(&self.selected) {
            let label = if self.install_jobs.contains_key(&self.selected) { "正在安装，离开页面后任务继续…" }
                else if progress.result.as_ref().is_some_and(Result::is_ok) { "安装已通过启动验证" } else { "安装未完成" };
            content = content.child(crate::i18n::text(label));
            if let Some(Err(error)) = &progress.result { content = content.child(div().text_color(cx.theme().danger).child(error.clone())); }
            let lines = div().id("cli-install-lines").max_h(px(180.)).overflow_y_scroll().v_flex().gap_1().text_xs().children(progress.lines.iter().cloned().map(|line| div().child(line)));
            content = content.child(div().max_h(px(180.)).child(crate::settings::widgets::scroll_faded("cli-install-log", lines)));
        }
        if let Some(error) = &self.info_error { content = content.child(div().text_color(cx.theme().danger).child(error.clone())); }
        if self.account_busy { content = content.child(crate::i18n::text("正在读取 CLI 账号…")); }
        if let Some(error) = &self.account_error { content = content.child(div().text_color(cx.theme().danger).child(error.clone())); }
        if let Some(account) = &self.account {
            content = content.child(account["account"]["email"].as_str().unwrap_or(crate::i18n::text("当前 CLI 登录账号")).to_string());
            content = content.child([account["account"]["type"].as_str(), account["account"]["plan"].as_str()].into_iter().flatten().collect::<Vec<_>>().join(" · "));
            if let Some(windows) = account["windows"].as_array() {
                for window in windows {
                    let used = window["usedPercent"].as_f64().filter(|used| used.is_finite());
                    let remaining = used.map(|used| format!("{:.0}%", (100. - used).clamp(0., 100.))).unwrap_or_else(|| "—".into());
                    let reset = window["resetsAt"].as_i64().and_then(|time| chrono::DateTime::from_timestamp(time, 0))
                        .map(|time| time.with_timezone(&chrono::Local).format("%m-%d %H:%M").to_string())
                        .or_else(|| window["resetsAt"].as_str().map(str::to_owned)).unwrap_or_else(|| crate::i18n::text("重置时间未知").into());
                    content = content.child(div().v_flex().gap_1()
                        .child(crate::ui_format!("{} · 剩余 {remaining}", "{} · {remaining} remaining", window["name"].as_str().unwrap_or("—")))
                        .child(div().text_sm().text_color(cx.theme().muted_foreground).child(reset)));
                }
            }
            if let Some(credits) = account["credits"].as_object() {
                let balance = if credits.get("unlimited").and_then(serde_json::Value::as_bool) == Some(true) { crate::i18n::text("不限量").to_string() }
                    else { credits.get("balance").filter(|value| !value.is_null()).map(|value| value.as_str().map(str::to_owned).unwrap_or_else(|| value.to_string())).unwrap_or_else(|| format!("{} / {} {}",
                        credits.get("used").filter(|value| !value.is_null()).map(ToString::to_string).unwrap_or_else(|| "—".into()),
                        credits.get("limit").filter(|value| !value.is_null()).map(ToString::to_string).unwrap_or_else(|| "—".into()),
                        credits.get("currency").and_then(serde_json::Value::as_str).unwrap_or(""))) };
                content = content.child(format!("{} · {balance}", credits.get("label").and_then(serde_json::Value::as_str).unwrap_or("Credits")));
            }
            if let Some(message) = account["unavailable"].as_str() { content = content.child(message.to_owned()); }
        }
        content
    }
    fn load(&mut self, save: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || self.install_jobs.contains_key(&self.selected) {
            return;
        }
        let before = self.configs.get(&self.selected).cloned();
        let mut after = before.clone().unwrap_or_default();
        if save {
            after.path = self.path.read(cx).value().trim().to_string();
            after.disabled = self.disabled;
            after.env.clear();
            for line in self
                .env
                .read(cx)
                .value()
                .lines()
                .filter(|line| !line.trim().is_empty())
            {
                let Some((key, value)) = line.split_once('=') else {
                    self.error = Some("环境变量每行使用 NAME=value 格式。".into());
                    cx.notify();
                    return;
                };
                after.env.push(CliEnvVar {
                    key: key.trim().into(),
                    value: value.into(),
                });
            }
            after.custom_models = self
                .models
                .read(cx)
                .value()
                .lines()
                .filter(|line| !line.trim().is_empty())
                .map(|line| {
                    let (id, label) = line.split_once('=').unwrap_or((line, ""));
                    CliCustomModel {
                        id: id.trim().into(),
                        label: label.trim().into(),
                    }
                })
                .collect();
        }
        self.busy = true;
        self.error = None;
        let directory = self.directory.clone();
        let id = self.selected.clone();
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    if save {
                        native_preferences::save(&directory, Change::Cli { id, before, after })
                    } else {
                        native_preferences::load(&directory)
                    }
                })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match result {
                    Ok(settings) => {
                        this.configs = settings.chat.external_cli_agents;
                        this.ready = true;
                        this.discovery_generation += 1;
                        this.discovery.clear();
                        this.discovery_tasks.clear();
                        for (id, _) in CLIS { this.detect((*id).into(), cx); }
                        this.select(this.selected.clone(), window, cx);
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }
}
impl Render for CliPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let english = crate::i18n::is_english();
        if self.english_placeholders != english {
            self.english_placeholders = english;
            self.path.update(cx, |input, cx| input.set_placeholder(crate::i18n::text("自动检测 PATH 中的命令"), window, cx));
        }
        let mut list = div()
            .flex()
            .flex_wrap()
            .gap_1()
            .p_1()
            .rounded(px(10.))
            .bg(cx.theme().group_box);
        // Disable switching while there is a draft, so it cannot silently disappear.
        let current = self
            .configs
            .get(&self.selected)
            .cloned()
            .unwrap_or_default();
        let dirty = self.has_unsaved_changes(cx);
        for &(id, label) in CLIS {
            list = list.child(
                Button::new(id)
                    .small()
                    .label(crate::i18n::display(label))
                    .ghost()
                    .selected(self.selected == id)
                    .disabled(self.busy || dirty && self.selected != id)
                    .on_click(
                        cx.listener(move |this, _, window, cx| this.select(id.into(), window, cx)),
                    ),
            );
        }
        let disabled = self.busy || !self.ready || self.install_jobs.contains_key(&self.selected);
        let mut detail = div()
            .v_flex()
            .gap_4()
            .flex_1()
            .min_w_0()
            .p_4()
            .rounded(px(12.))
            .bg(cx.theme().group_box)
            .child(
                div()
                    .text_size(px(14.))
                    .font_weight(FontWeight::MEDIUM)
                    .child(
                        CLIS.iter()
                            .find(|(id, _)| *id == self.selected)
                            .map(|(_, label)| *label)
                            .unwrap_or("CLI"),
                    ),
            )
            .child(
                Switch::new("cli-enabled")
                    .label(crate::i18n::text("启用此 CLI"))
                    .checked(!self.disabled)
                    .disabled(disabled)
                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                        this.disabled = !checked;
                        cx.notify();
                    })),
            )
            .child(crate::i18n::text("可执行文件路径"))
            .child(
                Input::new(&self.path)
                    .small()
                    .aria_label("CLI 可执行文件路径")
                    .disabled(disabled),
            )
            .child(crate::i18n::text("环境变量"))
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(crate::i18n::text("每行 NAME=value。此处包含的凭据只保存到本机配置。")),
            )
            .child(
                Textarea::new(&self.env)
                    .aria_label("CLI 环境变量")
                    .h(rems(9.))
                    .disabled(disabled),
            )
            .child(crate::i18n::text("自定义模型"))
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(crate::i18n::text("每行一个模型 ID，可使用 ID=显示名称。")),
            )
            .child(
                Textarea::new(&self.models)
                    .aria_label("CLI 自定义模型")
                    .h(rems(7.))
                    .disabled(disabled),
            );
        if !current.providers.is_empty() {
            detail = detail.child(div().text_base().child(crate::i18n::text("供应商配置")));
            for provider in &current.providers {
                detail = detail.child(div().text_sm().child(format!(
                    "{}{}",
                    provider.name,
                    if provider.id == current.current_provider {
                        " · 当前"
                    } else {
                        ""
                    }
                )));
            }
        }
        detail = detail.child(self.management_controls(cx));
        if let Some(error) = &self.error {
            detail = detail.child(div().text_color(cx.theme().danger).child(error.clone()));
        }
        detail = detail.child(div().flex().gap_2()
            .child(Button::new("cli-save").small().label(if self.busy { crate::i18n::text("保存中…") } else { crate::i18n::text("保存配置") }).primary().disabled(disabled).on_click(cx.listener(|this, _, window, cx| this.load(true, window, cx))))
            .child(Button::new("cli-reload").small().label(if dirty { crate::i18n::text("放弃修改") } else { crate::i18n::text("重新读取") }).disabled(self.busy).on_click(cx.listener(|this, _, window, cx| this.load(false, window, cx)))))
            .child(div().text_sm().text_color(cx.theme().muted_foreground).child(crate::i18n::text("检测、安装和账号查询使用已保存的配置。CLI 登录请按官方文档在终端完成；dsh 仍需单独初始化 Kivio profile。")));
        crate::kivio_settings::page_frame(
            "本地 CLI 管理",
            "管理各 CLI 的路径、环境变量和自定义模型。",
            div()
                .v_flex()
                .gap_5()
                .child(list)
                .child(detail)
                .into_any_element(),
            cx,
        )
    }
}
