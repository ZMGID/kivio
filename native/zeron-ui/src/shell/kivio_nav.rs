//! Kivio entries in Zeron's sidebar: primary destinations and list scope.

use super::*;

impl Shell {
    pub(super) fn render_kivio_sidebar_head(
        &mut self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let new_chat = Self::kivio_nav_row("kivio-nav-new", icons::PEN_NEW_SQUARE, "新建聊天", theme)
            .on_click(cx.listener(|this, _, _, cx| this.open_new_session(None, cx)));
        let search = self.kivio_search_button(theme, cx);
        let plugins = Self::kivio_nav_row("kivio-nav-plugins", icons::WIDGET, "插件", theme)
            .when(self.route == Route::Plugins, |el| {
                el.bg(crate::theme::glass_selected_bg()).text_color(theme.text)
            })
            .on_click(cx.listener(|this, _, _, cx| this.open_kivio_route(Route::Plugins, NavEntry::Plugins, cx)));
        let tasks = Self::kivio_nav_row("kivio-nav-tasks", icons::CHECKLIST, "任务", theme)
            .when(self.route == Route::Tasks, |el| {
                el.bg(crate::theme::glass_selected_bg()).text_color(theme.text)
            })
            .on_click(cx.listener(|this, _, _, cx| {
                this.suspend_file_images(cx);
                this.nav.push(NavEntry::Tasks);
                this.set_route(Route::Tasks, cx);
                cx.notify();
            }));
        let extensions = Self::kivio_nav_row("kivio-nav-extensions", icons::MAGIC_STICK_3, "扩展", theme)
            .when(self.route == Route::Extensions, |el| {
                el.bg(crate::theme::glass_selected_bg()).text_color(theme.text)
            })
            .on_click(cx.listener(|this, _, _, cx| this.open_kivio_route(Route::Extensions, NavEntry::Extensions, cx)));

        let by_project = self.settings.sidebar_organization == SidebarOrganization::ByProject;
        let scope = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(2.0))
            .child(
                Self::kivio_scope_chip("kivio-scope-recent", "最近", !by_project, theme).on_click(
                    cx.listener(|this, _, _, cx| {
                        this.set_sidebar_organization(SidebarOrganization::InOneList, cx)
                    }),
                ),
            )
            .child(
                Self::kivio_scope_chip("kivio-scope-sets", "集", false, theme)
                    .on_click(cx.listener(|this, _, _, cx| this.kivio_not_ported("集", cx))),
            )
            .child(
                Self::kivio_scope_chip("kivio-scope-projects", "项目", by_project, theme).on_click(
                    cx.listener(|this, _, _, cx| {
                        this.set_sidebar_organization(SidebarOrganization::ByProject, cx)
                    }),
                ),
            );

        div()
            .flex_none()
            .flex()
            .flex_col()
            .gap(px(2.0))
            .px(px(Theme::SPACE_SM))
            .pt(px(8.0))
            .pb(px(4.0))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(2.0))
                    .child(div().flex_1().min_w_0().child(new_chat))
                    .child(search),
            )
            .child(plugins)
            .child(tasks)
            .child(extensions)
            .child(div().pt(px(8.0)).child(scope))
            .into_any_element()
    }

    fn open_kivio_route(&mut self, route: Route, entry: NavEntry, cx: &mut Context<Self>) {
        self.suspend_file_images(cx);
        self.nav.push(entry);
        self.set_route(route, cx);
        cx.notify();
    }

    fn kivio_not_ported(&mut self, what: &str, cx: &mut Context<Self>) {
        self.show_notice(
            crate::toast::ToastKind::Info,
            format!("{what}尚未迁移到 GPUI 版本"),
            cx,
        );
    }

    // Same row anatomy as the project filter: 29px, 8px radius, 16px glyph slot.
    fn kivio_nav_row(
        id: &'static str,
        glyph: &'static str,
        label: &'static str,
        theme: &Theme,
    ) -> gpui_kit::Stateful<gpui_kit::Div> {
        div()
            .id(id)
            .debug_selector(|| id.into())
            .role(gpui_kit::Role::Button)
            .aria_label(label)
            .w_full()
            .h(px(29.0))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(Theme::SPACE_SM))
            .px(px(Theme::SPACE_SM))
            .rounded(px(8.0))
            .cursor_pointer()
            .text_size(crate::typography::ui_rems(13.0))
            .font_weight(gpui_kit::FontWeight::MEDIUM)
            .text_color(motion::hover_blend(id, theme.text.opacity(0.8), theme.text))
            .bg(motion::hover_blend(
                id,
                theme.glass_hover().opacity(0.0),
                theme.glass_hover(),
            ))
            .on_hover(motion::hover_listener(id))
            .child(popover::picker_icon_slot(
                icon(glyph)
                    .size(px(16.0))
                    .text_color(theme.text_muted)
                    .into_any_element(),
            ))
            .child(div().flex_1().min_w_0().child(SharedString::from(label)))
    }

    fn kivio_search_button(&mut self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        div()
            .id("kivio-nav-search")
            .role(gpui_kit::Role::Button)
            .aria_label("搜索")
            .size(px(26.0))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(6.0))
            .cursor_pointer()
            .bg(motion::hover_blend(
                "kivio-nav-search",
                theme.glass_hover().opacity(0.0),
                theme.glass_hover(),
            ))
            .on_hover(motion::hover_listener("kivio-nav-search"))
            .on_click(cx.listener(|this, _, window, cx| this.toggle_command_palette(window, cx)))
            .child(
                icon(icons::MAGNIFER)
                    .size(px(15.0))
                    .text_color(theme.text_muted),
            )
            .into_any_element()
    }

    fn kivio_scope_chip(
        id: &'static str,
        label: &'static str,
        active: bool,
        theme: &Theme,
    ) -> gpui_kit::Stateful<gpui_kit::Div> {
        div()
            .id(id)
            .debug_selector(|| id.into())
            .flex_none()
            .px(px(Theme::SPACE_SM))
            .h(px(22.0))
            .flex()
            .items_center()
            .rounded(px(6.0))
            .cursor_pointer()
            .text_size(crate::typography::ui_rems(12.0))
            .font_weight(gpui_kit::FontWeight::MEDIUM)
            .text_color(if active { theme.text } else { theme.text_muted })
            .bg(if active {
                crate::theme::glass_selected_bg()
            } else {
                theme.glass_hover().opacity(0.0)
            })
            .when(!active, |el| el.hover(|el| el.bg(theme.glass_hover())))
            .child(SharedString::from(label))
    }
}
