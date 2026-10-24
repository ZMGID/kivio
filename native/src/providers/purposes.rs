use super::{ModelChoice, ProviderSettings};
use crate::provider_settings::{
    DefaultModelSelection, DefaultModelsConfig, ModelProvider, sanitize_default_model_selection,
};
use gpui_kit::component::{
    ActiveTheme, Disableable, Sizable, StyledExt,
    button::Button,
    menu::{DropdownMenu, PopupMenuItem},
};
use gpui_kit::*;

#[derive(Clone, Copy, PartialEq)]
pub(super) enum Purpose {
    Chat,
    Vision,
    Video,
    Title,
    Compression,
    Image,
    Prompt,
    Advisor,
}
impl Purpose {
    const ALL: [Self; 8] = [
        Self::Chat,
        Self::Vision,
        Self::Video,
        Self::Title,
        Self::Compression,
        Self::Image,
        Self::Prompt,
        Self::Advisor,
    ];
    fn label(self) -> &'static str {
        match self {
            Self::Chat => "新对话默认模型",
            Self::Vision => "图片分析",
            Self::Video => "视频分析",
            Self::Title => "标题总结",
            Self::Compression => "上下文压缩",
            Self::Image => "图片生成",
            Self::Prompt => "问题优化",
            Self::Advisor => "顾问模型",
        }
    }
    fn fallback(self) -> &'static str {
        match self {
            Self::Chat => "未单独设置",
            Self::Video => "自动选择支持视频的模型",
            Self::Image => "主模型支持时跟随主模型",
            Self::Advisor => "关闭",
            _ => "跟随当前对话模型",
        }
    }
    fn selection(self, config: &DefaultModelsConfig) -> &DefaultModelSelection {
        match self {
            Self::Chat => &config.chat,
            Self::Vision => &config.vision,
            Self::Video => &config.video_analysis,
            Self::Title => &config.title_summary,
            Self::Compression => &config.compression,
            Self::Image => &config.image_generation,
            Self::Prompt => &config.prompt_optimize,
            Self::Advisor => &config.advisor,
        }
    }
    fn selection_mut(self, config: &mut DefaultModelsConfig) -> &mut DefaultModelSelection {
        match self {
            Self::Chat => &mut config.chat,
            Self::Vision => &mut config.vision,
            Self::Video => &mut config.video_analysis,
            Self::Title => &mut config.title_summary,
            Self::Compression => &mut config.compression,
            Self::Image => &mut config.image_generation,
            Self::Prompt => &mut config.prompt_optimize,
            Self::Advisor => &mut config.advisor,
        }
    }
    fn accepts(self, provider: &ModelProvider, model: &str) -> bool {
        if !matches!(self, Self::Vision | Self::Video | Self::Image) {
            return true;
        }
        let capabilities =
            kivio_model_runtime::model_metadata::capabilities_for_model(provider, model);
        match self {
            Self::Vision => capabilities.vision == Some(true),
            Self::Video => capabilities.video_input == Some(true),
            Self::Image => capabilities.image_generation == Some(true),
            _ => true,
        }
    }
}

pub(super) fn sanitize(defaults: &mut DefaultModelsConfig, providers: &[ModelProvider]) {
    for purpose in Purpose::ALL {
        sanitize_default_model_selection(purpose.selection_mut(defaults), providers);
    }
}

impl ProviderSettings {
    fn select_purpose(
        &mut self,
        purpose: Purpose,
        provider_id: String,
        model: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy || self.dirty || !self.ready {
            return;
        }
        if !provider_id.is_empty() {
            match self.selected(&provider_id, &model) {
                Ok((provider, _)) if purpose.accepts(&provider, &model) => {}
                Ok(_) => {
                    self.error = Some("此模型不具备该用途需要的能力。".into());
                    cx.notify();
                    return;
                }
                Err(error) => {
                    self.error = Some(error);
                    cx.notify();
                    return;
                }
            }
        }
        let mut next = self.confirmed.clone();
        next.select_purpose(purpose, DefaultModelSelection { provider_id, model });
        self.persist(next, window, cx);
    }

    fn disable_video(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || self.dirty || !self.ready { return; }
        let mut next = self.confirmed.clone();
        next.video_enabled = false;
        self.persist(next, window, cx);
    }

    pub(super) fn render_purposes(&self, cx: &Context<Self>) -> AnyElement {
        let mut content = div()
            .v_flex()
            .gap_0()
            .rounded(px(12.))
            .bg(cx.theme().group_box)
            .px_4()
            .py_3()
            .child(
                div()
                    .text_size(px(13.))
                    .font_weight(FontWeight::MEDIUM)
                    .child(crate::i18n::text("模型分工")),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(crate::i18n::text("按原设置保存各用途；未单独配置的用途沿用现有继承规则。")),
            );
        for purpose in Purpose::ALL
            .into_iter()
            .filter(|purpose| *purpose != Purpose::Chat)
        {
            let current = purpose.selection(&self.confirmed.defaults);
            let selected = (current.provider_id.clone(), current.model.clone());
            let choices: Vec<ModelChoice> = self
                .models()
                .into_iter()
                .filter(|choice| {
                    self.confirmed
                        .providers
                        .iter()
                        .find(|provider| provider.id == choice.provider_id)
                        .is_some_and(|provider| purpose.accepts(provider, &choice.model))
                })
                .collect();
            let enabled = purpose != Purpose::Video || self.confirmed.video_enabled;
            let label = choices
                .iter()
                .find(|choice| choice.value == selected)
                .map(|choice| choice.label.clone())
                .unwrap_or_else(|| {
                    if current.provider_id.is_empty() {
                        purpose.fallback().into()
                    } else {
                        format!("{} · {}", current.provider_id, current.model).into()
                    }
                });
            let label = if enabled { label } else { "关闭".into() };
            let owner = cx.weak_entity();
            content = content.child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .min_h(px(60.))
                    .border_b_1()
                    .border_color(cx.theme().border.opacity(0.6))
                    .child(div().text_size(px(13.)).child(purpose.label()))
                    .child(
                        Button::new(SharedString::from(format!("purpose-{}", purpose.label())))
                            .small()
                            .label(crate::i18n::display(label))
                            .dropdown_caret(true)
                            .disabled(self.busy || self.dirty)
                            .dropdown_menu(move |mut menu, _, _| {
                                if purpose == Purpose::Video {
                                    let disable_owner = owner.clone();
                                    menu = menu.item(PopupMenuItem::new(crate::i18n::text("关闭视频分析")).checked(!enabled)
                                        .on_click(move |_, window, cx| { let _ = disable_owner.update(cx, |this, cx| this.disable_video(window, cx)); }));
                                }
                                let clear_owner = owner.clone();
                                menu = menu.item(
                                    PopupMenuItem::new(purpose.fallback())
                                        .checked(enabled && selected.0.is_empty())
                                        .on_click(move |_, window, cx| {
                                            let _ = clear_owner.update(cx, |this, cx| {
                                                this.select_purpose(
                                                    purpose,
                                                    String::new(),
                                                    String::new(),
                                                    window,
                                                    cx,
                                                )
                                            });
                                        }),
                                );
                                for choice in &choices {
                                    let choice = choice.clone();
                                    let owner = owner.clone();
                                    menu = menu.item(
                                        PopupMenuItem::new(choice.label.clone())
                                            .checked(enabled && choice.value == selected)
                                            .on_click(move |_, window, cx| {
                                                let _ = owner.update(cx, |this, cx| {
                                                    this.select_purpose(
                                                        purpose,
                                                        choice.provider_id.clone(),
                                                        choice.model.clone(),
                                                        window,
                                                        cx,
                                                    )
                                                });
                                            }),
                                    );
                                }
                                menu
                            }),
                    ),
            );
        }
        content.into_any_element()
    }
}

impl super::ProviderDocument {
    pub(super) fn select_purpose(&mut self, purpose: Purpose, selection: DefaultModelSelection) {
        if purpose == Purpose::Video { self.video_enabled = true; }
        *purpose.selection_mut(&mut self.defaults) = selection;
    }
}
