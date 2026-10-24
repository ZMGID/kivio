//! Plugin marketplace and extension pages, built from Zeron's page primitives.
//! UI-only preview: never infer an empty library from an unconnected data owner.
use crate::composer::{ComposerInput, ComposerInputEvent};
use crate::icons::{self, icon};
use crate::motion;
use crate::settings::widgets;
use crate::theme::Theme;
use gpui_kit::component::{
    Disableable,
    button::{Button, ButtonCustomVariant, ButtonVariants},
};
use gpui_kit::{prelude::*, *};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Plugins,
    Extensions,
}

struct Section {
    label: &'static str,
    icon: &'static str,
    search: &'static str,
    action: &'static str,
    filters: &'static [&'static str],
    empty_hint: &'static str,
}

const PLUGINS: [Section; 3] = [
    Section {
        label: "插件",
        icon: icons::WIDGET,
        search: "搜索插件",
        action: "导入插件",
        filters: &["全部", "已安装"],
        empty_hint: "插件可以一次带来技能、MCP 服务和命令。",
    },
    Section {
        label: "Skill",
        icon: icons::MAGIC_STICK_3,
        search: "搜索 Skill",
        action: "新建 Skill",
        filters: &["已安装", "商店"],
        empty_hint: "Skill 是带说明的可复用能力，模型会在需要时按说明调用。",
    },
    Section {
        label: "MCP",
        icon: icons::REMOTE_SERVER,
        search: "搜索 MCP 服务",
        action: "添加服务",
        filters: &["已安装", "注册表"],
        empty_hint: "MCP 服务为模型提供外部工具与数据。",
    },
];

const EXTENSIONS: [Section; 4] = [
    Section {
        label: "助手",
        icon: icons::BOT,
        search: "搜索助手",
        action: "新建助手",
        filters: &["全部", "我的"],
        empty_hint: "助手保存一套系统提示词、模型与工具偏好，随时切换。",
    },
    Section {
        label: "知识库",
        icon: icons::FOLDER_WITH_FILES,
        search: "搜索知识库",
        action: "新建知识库",
        filters: &["全部"],
        empty_hint: "导入文档后，对话中可以检索其中的内容。",
    },
    Section {
        label: "媒体",
        icon: icons::GALLERY,
        search: "搜索作品",
        action: "新建生成",
        filters: &["全部", "图片", "视频"],
        empty_hint: "在这里生成图片与视频，作品会保存在此处。",
    },
    Section {
        label: "笔记",
        icon: icons::DOCUMENT,
        search: "搜索笔记",
        action: "新建笔记",
        filters: &["全部"],
        empty_hint: "笔记是 Markdown 文件，也可以从对话中保存。",
    },
];

pub struct ExtensionCenter {
    kind: Kind,
    section: usize,
    filter: usize,
    search: Entity<ComposerInput>,
    tab_focus: Vec<FocusHandle>,
    filter_focus: Vec<FocusHandle>,
    _search: Subscription,
}

impl ExtensionCenter {
    pub fn new(kind: Kind, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| {
            ComposerInput::with_context(PLUGINS[0].search, "PaletteSearch", cx)
                .with_single_line()
                .with_tab_stop()
                .with_accessibility_role(Role::SearchInput)
        });
        let subscription = cx.subscribe(&search, |_, _, event, cx| {
            if matches!(event, ComposerInputEvent::Edited) {
                cx.notify();
            }
        });
        let mut this = Self {
            kind,
            section: 0,
            filter: 0,
            search,
            tab_focus: (0..4).map(|_| cx.focus_handle().tab_stop(true)).collect(),
            filter_focus: (0..3).map(|_| cx.focus_handle().tab_stop(true)).collect(),
            _search: subscription,
        };
        this.reset_section(0, cx);
        this
    }

    pub fn show(&mut self, kind: Kind, cx: &mut Context<Self>) {
        if self.kind != kind {
            self.kind = kind;
            self.reset_section(0, cx);
        }
    }

    fn sections(&self) -> &'static [Section] {
        match self.kind {
            Kind::Plugins => &PLUGINS,
            Kind::Extensions => &EXTENSIONS,
        }
    }

    fn reset_section(&mut self, index: usize, cx: &mut Context<Self>) {
        self.section = index;
        self.filter = 0;
        let placeholder = self.sections()[index].search;
        self.search.update(cx, |input, cx| {
            input.set_text("", cx);
            input.set_placeholder(placeholder, cx);
        });
        cx.notify();
    }

    fn render_tabs(
        &self,
        sections: &'static [Section],
        theme: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let mut row = div().flex().flex_row().items_center().gap(px(4.0));
        for (index, section) in sections.iter().enumerate() {
            let selected = index == self.section;
            let key = format!("extension-tab-{}", section.label);
            let hover_key = format!("{key}-hover");
            let selection_t = widgets::tab_selection_t(
                window,
                format!("{key}-selection"),
                selected,
                motion::reduced_motion(cx),
            );
            let glyph = motion::hover_blend(
                &hover_key,
                motion::mix(theme.text_muted, theme.text, selection_t),
                theme.text,
            );
            row = row.child(
                widgets::section_tab(theme, selected, selection_t, key, hover_key)
                    .role(Role::Tab)
                    .aria_label(section.label)
                    .aria_selected(selected)
                    .track_focus(&self.tab_focus[index].clone().tab_stop(selected))
                    .focus_visible(|el| el.border_2().border_color(theme.accent))
                    .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                        let count = this.sections().len();
                        let next = match event.keystroke.key.as_str() {
                            "right" => (index + 1) % count,
                            "left" => (index + count - 1) % count,
                            "home" => 0,
                            "end" => count - 1,
                            "enter" | "space" => index,
                            _ => return,
                        };
                        this.reset_section(next, cx);
                        window.focus(&this.tab_focus[next], cx);
                        cx.stop_propagation();
                    }))
                    .on_click(cx.listener(move |this, _, _, cx| this.reset_section(index, cx)))
                    .child(icon(section.icon).size(px(16.0)).text_color(glyph))
                    .child(section.label),
            );
        }
        row
    }

    fn render_search(&self, theme: &Theme) -> Div {
        div()
            .w(px(220.0))
            .flex_none()
            .h(px(32.0))
            .px(px(10.0))
            .rounded(px(8.0))
            .bg(widgets::select_fill(theme, false))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.0))
            .text_size(crate::typography::ui_rems(13.0))
            .child(
                icon(icons::MAGNIFER)
                    .size(px(14.0))
                    .flex_none()
                    .text_color(theme.text_muted),
            )
            .child(div().flex_1().min_w_0().child(self.search.clone()))
    }

    fn render_filters(&self, section: &Section, theme: &Theme, cx: &mut Context<Self>) -> Div {
        let mut row = div().flex().flex_row().items_center().gap(px(2.0));
        for (index, label) in section.filters.iter().enumerate() {
            let active = index == self.filter;
            row = row.child(
                div()
                    .id(SharedString::from(format!("extension-filter-{index}")))
                    .flex_none()
                    .px(px(Theme::SPACE_SM))
                    .h(px(22.0))
                    .flex()
                    .items_center()
                    .rounded(px(6.0))
                    .role(Role::Tab)
                    .aria_label(*label)
                    .aria_selected(active)
                    .track_focus(&self.filter_focus[index].clone().tab_stop(active))
                    .focus_visible(|el| el.border_2().border_color(theme.accent))
                    .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                        let count = this.sections()[this.section].filters.len();
                        let next = match event.keystroke.key.as_str() {
                            "right" => (index + 1) % count,
                            "left" => (index + count - 1) % count,
                            "home" => 0,
                            "end" => count - 1,
                            "enter" | "space" => index,
                            _ => return,
                        };
                        this.filter = next;
                        window.focus(&this.filter_focus[next], cx);
                        cx.notify();
                        cx.stop_propagation();
                    }))
                    .text_size(crate::typography::ui_rems(12.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(if active { theme.text } else { theme.text_muted })
                    .bg(if active {
                        crate::theme::glass_selected_bg()
                    } else {
                        theme.glass_hover().opacity(0.0)
                    })
                    .when(!active, |el| el.hover(|el| el.bg(theme.glass_hover())))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.filter = index;
                        cx.notify();
                    }))
                    .child(SharedString::from(*label)),
            );
        }
        row
    }

    fn render_rows(&self, section: &Section, theme: &Theme) -> Div {
        widgets::section_card(theme).mt_0().child(
            div()
                .py_12()
                .px_6()
                .flex()
                .flex_col()
                .items_center()
                .gap_2()
                .child(icon(section.icon).size_6().text_color(theme.text_muted))
                .child(widgets::row_title(theme, format!("{}尚未接入", section.label)).mt_1())
                .child(
                    div()
                        .max_w(rems(24.0))
                        .text_center()
                        .text_size(crate::typography::ui_rems(widgets::ROW_DESCRIPTION_SIZE))
                        .text_color(theme.text_muted)
                        .child(section.empty_hint),
                )
                .child(
                    div()
                        .mt_2()
                        .text_size(crate::typography::ui_rems(11.0))
                        .text_color(theme.text_faint)
                        .child("当前仅预览界面，不读取或修改已有数据"),
                ),
        )
    }
}

impl Render for ExtensionCenter {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).for_settings_surface();
        let sections = self.sections();
        let section = &sections[self.section.min(sections.len() - 1)];
        let (title, subtitle, scroll_key) = match self.kind {
            Kind::Plugins => (
                "插件市场",
                "为 Kivio 安装技能、MCP 服务和插件。",
                "plugin-center-scroll",
            ),
            Kind::Extensions => (
                "扩展",
                "助手、知识库、媒体与笔记。",
                "extension-center-scroll",
            ),
        };

        let action = Button::new("extension-primary-action")
            .custom(
                ButtonCustomVariant::new(cx)
                    .color(widgets::select_fill(&theme, false))
                    .foreground(theme.text)
                    .hover(widgets::select_fill(&theme, true)),
            )
            .label(format!("{}…", section.action))
            .disabled(true)
            .tooltip("此功能尚未接入，已有数据不受影响");

        let header = div()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .gap(px(16.0))
            .child(widgets::page_header(&theme, title, None))
            .child(action);

        let tabs = self.render_tabs(sections, &theme, window, cx);
        let controls = div().mt_6().child(tabs);
        let filters = div()
            .mt_4()
            .flex()
            .flex_wrap()
            .items_center()
            .justify_between()
            .gap_3()
            .child(self.render_filters(section, &theme, cx))
            .child(self.render_search(&theme));
        let rows = div().mt(px(8.0)).child(self.render_rows(section, &theme));

        let page = widgets::page_column()
            .child(header)
            .child(widgets::page_subtitle(&theme, subtitle))
            .child(controls)
            .child(filters)
            .child(rows);
        let area = div()
            .id(scroll_key)
            .size_full()
            .overflow_y_scroll()
            .text_color(theme.text)
            .child(page);
        widgets::scroll_faded(scroll_key, area)
    }
}
