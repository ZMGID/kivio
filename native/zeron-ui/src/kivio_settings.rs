//! Kivio's settings information architecture in the imported Zeron shell.
//! Unsupported host features remain visibly read-only, never successful no-op controls.
use crate::{settings::widgets, shell::SettingsSection, theme::Theme, typography};
use gpui_kit::component::{
    Disableable, Selectable, Sizable, StyledExt,
    button::Button,
    input::{Input, InputState, Textarea, TextareaState},
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
    let theme = Theme::of(cx).clone();
    widgets::scroll_faded(
        format!("kivio-settings-scroll-{title}"),
        div()
            .id(SharedString::from(format!(
                "kivio-settings-content-{title}"
            )))
            .size_full()
            .overflow_y_scroll()
            .child(
                div()
                    .w_full()
                    .max_w(px(1040.))
                    .mx_auto()
                    .px(px(32.))
                    .pt(px(24.))
                    .pb(px(32.))
                    .text_size(px(14.))
                    .child(widgets::page_header(&theme, title, None))
                    .child(widgets::page_subtitle(&theme, subtitle.to_owned()))
                    .child(div().mt(px(20.)).child(content)),
            ),
    )
    .into_any_element()
}

pub struct SettingsPage {
    pub section: SettingsSection,
    settings: Option<Settings>,
    directory: std::path::PathBuf,
    busy: bool,
    prompts: std::collections::HashMap<TextPreference, Entity<TextareaState>>,
    name: Option<Entity<InputState>>,
    error: Option<String>,
    _load: Task<()>,
}
impl SettingsPage {
    pub fn new(directory: std::path::PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Self {
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
                            ];
                            this.prompts = fields
                                .into_iter()
                                .map(|(field, text)| {
                                    let text = if text.is_empty() {
                                        default_prompt(field)
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
                            this.settings = Some(settings);
                        }
                        Err(error) => this.error = Some(error),
                    };
                    cx.notify();
                })
            });
        });
        Self {
            section: SettingsSection::General,
            settings: None,
            directory,
            busy: false,
            prompts: Default::default(),
            name: None,
            error: None,
            _load: task,
        }
    }
    fn save(&mut self, change: Change, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.error = None;
        let directory = self.directory.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move { native_preferences::save(&directory, change) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                match result {
                    Ok(settings) => this.settings = Some(settings),
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}

fn row(label: impl Into<SharedString>, control: AnyElement, theme: &Theme) -> Div {
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(24.))
        .py(px(10.))
        .border_b_1()
        .border_color(theme.border)
        .child(div().flex_1().min_w_0().child(label.into()))
        .child(div().flex_none().max_w(px(300.)).child(control))
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
            .accessibility_label(label)
            .checked(checked)
            .disabled(true)
            .into_any_element(),
        theme,
    )
}
fn card(title: &'static str, rows: Vec<Div>, theme: &Theme) -> Div {
    div()
        .bg(theme.surface_card)
        .border_1()
        .border_color(theme.border)
        .rounded(px(12.))
        .p(px(16.))
        .flex()
        .flex_col()
        .child(div().text_color(theme.text_muted).mb(px(8.)).child(title))
        .children(rows)
}
fn unavailable(theme: &Theme) -> Div {
    div()
        .text_color(theme.text_muted)
        .text_size(px(13.))
        .child("部分功能尚未迁移，灰色选项和只读信息暂不可修改。")
}
fn font_picker(code: bool, cx: &App) -> AnyElement {
    let current = if code {
        typography::code_requested(cx)
    } else {
        typography::requested(cx)
    };
    let choices = typography::availability(cx).choices().to_vec();
    Button::new(if code { "code-font" } else { "ui-font" })
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
        let theme = Theme::of(cx).clone();
        let Some(s) = &self.settings else {
            return page_frame(
                self.section.label(),
                "",
                div()
                    .child(self.error.clone().unwrap_or("正在读取 Kivio 设置…".into()))
                    .into_any_element(),
                cx,
            );
        };
        let mut body = div().flex().flex_col().gap(px(20.));
        if let Some(error) = &self.error {
            body = body.child(div().child(format!("保存失败：{error}")));
        }

        if let SettingsSection::Kivio(
            section @ (Section::Translate | Section::Lens | Section::Chat),
        ) = self.section
        {
            return self.render_detail(section, &theme, cx);
        }
        let subtitle;
        match self.section {
            SettingsSection::General => {
                subtitle = "外观、行为、归档和权限。";
                let size = typography::font_size(cx);
                let size_picker = Button::new("ui-size")
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
                            value("界面语言", "中文（设置页面）", &theme),
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
                            pending("开机启动", s.launch_at_startup, &theme),
                            pending("启动后最小化到托盘", s.launch_minimized_to_tray, &theme),
                            pending("关闭时保持聊天窗口", s.keep_chat_window_alive, &theme),
                            pending("回复完成通知", s.chat_completion_notifications, &theme),
                            row(
                                "自动重试",
                                Switch::new("retry-enabled")
                                    .accessibility_label("自动重试")
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
                                Button::new("retry-count")
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
                    .child(card(
                        "首次使用",
                        vec![value("设置引导", "原生引导尚未接入", &theme)],
                        &theme,
                    ));
            }
            SettingsSection::Appearance => {
                subtitle = "管理明暗色板，预览、编辑与分享自定义主题。";
                let current = crate::appearance::mode(cx);
                let modes = [
                    (crate::appearance::AppearanceMode::System, "跟随系统"),
                    (crate::appearance::AppearanceMode::Light, "浅色"),
                    (crate::appearance::AppearanceMode::Dark, "深色"),
                ];
                let label = modes.iter().find(|(mode, _)| *mode == current).unwrap().1;
                body = body
                    .child(card(
                        "外观",
                        vec![row(
                            "明暗模式",
                            Button::new("theme-mode")
                                .label(label)
                                .dropdown_caret(true)
                                .dropdown_menu(move |mut menu, _, _| {
                                    for (mode, label) in modes {
                                        menu = menu.item(
                                            PopupMenuItem::new(label)
                                                .checked(mode == current)
                                                .on_click(move |_, _, cx| {
                                                    crate::appearance::set_mode(mode, cx)
                                                }),
                                        );
                                    }
                                    menu
                                })
                                .into_any_element(),
                            &theme,
                        )],
                        &theme,
                    ))
                    .child(theme_library_grid(cx));
            }
            SettingsSection::Shortcuts => {
                subtitle = "集中管理所有全局快捷键。";
                body = body.child(unavailable(&theme)).child(card(
                    "快捷键",
                    vec![
                        value("输入翻译", s.hotkey.clone(), &theme),
                        value("打开聊天", s.chat_hotkey.clone(), &theme),
                        value("关闭聊天", s.close_chat_hotkey.clone(), &theme),
                        value("截图翻译", s.screenshot_translation.hotkey.clone(), &theme),
                        value(
                            "选中文本翻译",
                            s.screenshot_translation.text_hotkey.clone(),
                            &theme,
                        ),
                        value(
                            "替换翻译",
                            s.screenshot_translation.replace_hotkey.clone(),
                            &theme,
                        ),
                        value("截图标注", s.screenshot_annotate.hotkey.clone(), &theme),
                        value("Lens", s.lens.hotkey.clone(), &theme),
                    ],
                    &theme,
                ));
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
                if section != Section::About {
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
                            format!("{} 个已有配置", s.chat.external_cli_agents.len()),
                            &theme,
                        ),
                    ],
                    Section::Computer => vec![
                        value("桌面操控", "原生权限与执行器尚未接入", &theme),
                        value("浏览器操控", "浏览器扩展连接尚未接入", &theme),
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
                        value("应用", "Kivio Native", &theme),
                        value("界面", "Zeron / GPUI Kit 0.8.0", &theme),
                        value("数据", "独立原生预览配置", &theme),
                    ],
                    Section::Mixer => vec![],
                };
                body = body.child(card(section.label(), rows, &theme));
            }
            _ => {
                subtitle = "";
            }
        }
        page_frame(self.section.label(), subtitle, body.into_any_element(), cx)
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
            .label(label)
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
                                    this.save(Change::Text(field, value.clone()), cx)
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
        div()
            .flex()
            .flex_col()
            .gap_2()
            .py_3()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .child(title)
                    .child(
                        Button::new(SharedString::from(format!("reset-{field:?}")))
                            .small()
                            .label("恢复默认")
                            .disabled(self.busy)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                if let Some(input) = this.prompts.get(&field) {
                                    input.update(cx, |input, cx| {
                                        input.set_value(default_prompt(field), window, cx)
                                    });
                                }
                                this.save(Change::Text(field, String::new()), cx);
                            })),
                    ),
            )
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(theme.text_muted)
                    .child(hint),
            )
            .child(Textarea::new(&input).aria_label(title).h(px(132.)))
            .child(
                div().flex().justify_end().child(
                    Button::new(SharedString::from(format!("save-{field:?}")))
                        .small()
                        .label("保存提示词")
                        .disabled(self.busy)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(input) = this.prompts.get(&field) {
                                let value = input.read(cx).value().to_string();
                                this.save(Change::Text(field, value), cx);
                            }
                        })),
                ),
            )
    }

    fn render_detail(&self, section: Section, theme: &Theme, cx: &Context<Self>) -> AnyElement {
        let s = self.settings.as_ref().unwrap();
        let mut body = div().flex().flex_col().gap_3();
        if let Some(error) = &self.error {
            body = body.child(div().child(format!("保存失败：{error}")));
        }
        let subtitle;
        match section {
            Section::Translate => {
                subtitle = "输入翻译与快速翻译的语言、模型、OCR 和提示词。";
                body = body
                    .child(card("翻译语言", vec![
                        described_row("阅读目标语言", "你想看的语言，用于划词、截图和替换翻译。", self.language(TextPreference::ReadingLanguage, &s.reading_target_lang, false, cx), theme),
                        described_row("输入翻译输出语言", "你准备发送的语言；自动仅用于中英互译。", self.language(TextPreference::InputLanguage, &s.target_lang, false, cx), theme),
                    ], theme))
                    .child(card("输入翻译", vec![
                        value("模型组合", format!("{} · {}", s.translator_provider_id, s.translator_model), theme),
                        self.prompt_editor(TextPreference::InputPrompt, "输入翻译提示词", "使用 {lang} 和 {text} 插入目标语言与原文；留空使用内置模板。", theme, cx),
                    ], theme))
                    .child(card("快速翻译 · 选项", vec![
                        pending("启用快速翻译", s.screenshot_translation.enabled, theme),
                        pending("显示原文", !s.screenshot_translation.direct_translate, theme),
                        pending("思考模式", s.screenshot_translation.thinking_enabled, theme),
                        pending("流式输出", s.screenshot_translation.stream_enabled, theme),
                        pending("截图后保持全屏覆盖", s.screenshot_translation.keep_fullscreen_after_capture, theme),
                        value("翻译卡宽度", format!("{} px", s.screenshot_translation.card_width), theme),
                    ], theme))
                    .child(card("替换翻译", vec![
                        pending("启用替换翻译", s.screenshot_translation.replace_enabled, theme),
                        value("替换翻译离线包", if s.screenshot_translation.rapid_ocr_tier == "high" { "高精度 · PP-OCRv6" } else { "标准 · PP-OCRv5" }, theme),
                    ], theme))
                    .child(card("截图翻译", vec![
                        value("截图翻译处理方式", match s.screenshot_translation.ocr_mode { Some(kivio_model_runtime::settings_config::OcrMode::System) => "系统 OCR", Some(kivio_model_runtime::settings_config::OcrMode::RapidOcr) => "RapidOCR", _ => "视觉模型" }, theme),
                        div().text_size(px(12.)).text_color(theme.text_muted).child("替换翻译始终使用 RapidOCR。截图与全局热键的平台入口尚未接入原生窗口。"),
                        value("模型组合", format!("{} · {}", s.screenshot_translation.provider_id, s.screenshot_translation.model), theme),
                    ], theme))
                    .child(card("自定义提示词", vec![
                        self.prompt_editor(TextPreference::ScreenshotPrompt, "截图翻译提示词", "留空使用内置截图翻译模板。", theme, cx),
                        self.prompt_editor(TextPreference::TextPrompt, "选中文本翻译提示词", "独立于截图模板，适用于干净的选中文本。", theme, cx),
                        self.prompt_editor(TextPreference::ReplacementPrompt, "替换翻译提示词", "仅自定义翻译规则，保留原有 JSON 输出约定。", theme, cx),
                    ], theme));
            }
            Section::Lens => {
                subtitle = "视觉问答的响应方式、对话与提示词。";
                body = body
                    .child(card(
                        "Lens 配置",
                        vec![
                            pending("启用 Lens", s.lens.enabled, theme),
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
                            pending("流式输出", s.lens.stream_enabled, theme),
                            pending("思考模式", s.lens.thinking_enabled, theme),
                        ],
                        theme,
                    ))
                    .child(card(
                        "对话",
                        vec![
                            pending("发送到 Kivio Agent", s.lens.send_to_chat, theme),
                            value(
                                "消息顺序",
                                if s.lens.message_order == "desc" {
                                    "从新到旧"
                                } else {
                                    "从旧到新"
                                },
                                theme,
                            ),
                            pending("截图选择提示", s.lens.show_capture_hint, theme),
                        ],
                        theme,
                    ))
                    .child(card(
                        "翻译引擎",
                        vec![value(
                            "模型组合",
                            if s.lens.model.is_empty() {
                                "跟随输入翻译".to_string()
                            } else {
                                format!("{} · {}", s.lens.provider_id, s.lens.model)
                            },
                            theme,
                        )],
                        theme,
                    ))
                    .child(card(
                        "截图自动归档",
                        vec![
                            pending("截图自动归档", s.image_archive_enabled, theme),
                            value(
                                "归档路径",
                                if s.image_archive_path.is_empty() {
                                    "系统默认".to_string()
                                } else {
                                    s.image_archive_path.clone()
                                },
                                theme,
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
                                .gap_2()
                                .child(Input::new(name).aria_label("用户名"))
                                .child(
                                    Button::new("save-display-name")
                                        .small()
                                        .label("保存")
                                        .disabled(self.busy)
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            if let Some(name) = &this.name {
                                                let value = name.read(cx).value().to_string();
                                                this.save(
                                                    Change::Text(TextPreference::UserName, value),
                                                    cx,
                                                );
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
                                format!("{} 个配置", s.chat.external_cli_agents.len()),
                                theme,
                            ),
                            div().text_size(px(12.)).text_color(theme.text_muted).child(
                                "原生 Agent 的工具执行循环尚未接入；配置状态不代表工具已运行。",
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
                            pending("联网搜索", s.chat.chat_mode.web_search, theme),
                            pending("读取网页", s.chat.chat_mode.web_fetch, theme),
                            pending("知识库检索", s.chat.chat_mode.knowledge_search, theme),
                            pending("记忆只读", s.chat.chat_mode.memory_tools, theme),
                            pending("只读 MCP", s.chat.chat_mode.mcp_read_only, theme),
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
        .items_center()
        .justify_between()
        .gap_4()
        .py_3()
        .border_b_1()
        .border_color(theme.border)
        .child(
            div().flex_1().min_w_0().child(label).child(
                div()
                    .mt_1()
                    .text_size(px(12.))
                    .text_color(theme.text_muted)
                    .child(description),
            ),
        )
        .child(div().flex_none().child(control))
}

fn default_prompt(field: TextPreference) -> String {
    use kivio_model_runtime::{prompts, settings_config};
    match field {
        TextPreference::InputPrompt => prompts::DEFAULT_TRANSLATION_TEMPLATE.into(),
        TextPreference::ScreenshotPrompt => prompts::DEFAULT_SCREENSHOT_TRANSLATION_TEMPLATE.into(),
        TextPreference::TextPrompt => prompts::DEFAULT_SELECTED_TEXT_TRANSLATION_TEMPLATE.into(),
        TextPreference::ReplacementPrompt => prompts::DEFAULT_REPLACE_TRANSLATION_TEMPLATE.into(),
        TextPreference::LensSystemPrompt => settings_config::default_lens_system_prompt("zh", true),
        TextPreference::LensQuestionPrompt => settings_config::default_question_prompt("zh", true),
        TextPreference::AgentPrompt => settings_config::default_chat_system_prompt(false),
        _ => String::new(),
    }
}

fn theme_library_grid(cx: &App) -> Div {
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
        grid = grid.child(
            div()
                .w(px(180.))
                .flex()
                .flex_col()
                .gap_2()
                .child(previews)
                .child(
                    Button::new(SharedString::from(format!("theme-{id}")))
                        .label(if selected {
                            format!("{label} · 已应用")
                        } else {
                            label.to_string()
                        })
                        .selected(selected)
                        .w_full()
                        .on_click(move |_, _, cx| {
                            crate::appearance::set_theme(
                                crate::theme::Appearance::Light,
                                light.clone(),
                                cx,
                            );
                            crate::appearance::set_theme(
                                crate::theme::Appearance::Dark,
                                dark.clone(),
                                cx,
                            );
                            crate::appearance::set_accent(
                                zeron_theme::AccentSelection::ThemeDefault,
                                cx,
                            );
                        }),
                ),
        );
    }
    div().flex().flex_col().gap_3().child("主题库").child(grid)
}
fn theme_miniature(variant: &zeron_theme::ThemeVariant) -> Div {
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
