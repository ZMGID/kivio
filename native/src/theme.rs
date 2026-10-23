use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::{App, px, rgb};

pub fn apply_system(cx: &mut App) {
    Theme::sync_system_appearance(None, cx);
    apply_palette(cx);
}

pub fn set_mode(mode: ThemeMode, cx: &mut App) {
    Theme::change(mode, None, cx);
    apply_palette(cx);
}

fn apply_palette(cx: &mut App) {
    // Product palette from DESIGN.md; values live only at this shared theme boundary.
    Theme::update(cx, |theme| {
        let dark = theme.is_dark();
        theme.background = rgb(if dark { 0x212121 } else { 0xfdfcfa }).into();
        theme.foreground = rgb(if dark { 0xe8e8ed } else { 0x1d1d1f }).into();
        theme.muted = rgb(if dark { 0x2a2a2d } else { 0xf4f3f1 }).into();
        theme.muted_foreground = rgb(if dark { 0xb0b0b8 } else { 0x696969 }).into();
        theme.border = rgb(if dark { 0x3a3a3e } else { 0xe2e1df }).into();
        theme.primary = rgb(if dark { 0x6694ff } else { 0x2f6ff0 }).into();
        theme.ring = theme.primary.opacity(0.35);
        theme.button_primary = theme.primary;
        theme.button_primary_hover = rgb(if dark { 0x7da4ff } else { 0x2563db }).into();
        theme.button_primary_active = rgb(if dark { 0x4b7de8 } else { 0x1d54c5 }).into();
        theme.button_primary_foreground = rgb(0xffffff).into();
        theme.primary_foreground = theme.button_primary_foreground;
        theme.sidebar = rgb(if dark { 0x262629 } else { 0xf9f8f6 }).into();
        theme.sidebar_foreground = theme.foreground;
        theme.sidebar_border = theme.border;
        theme.sidebar_accent = rgb(if dark { 0x36363c } else { 0xeeedeb }).into();
        theme.sidebar_accent_foreground = theme.foreground;
        theme.sidebar_primary = theme.primary;
        theme.sidebar_primary_foreground = theme.primary_foreground;
        // Keep Kit menus, inputs, tabs and buttons in the same neutral world.
        theme.accent = theme.sidebar_accent;
        theme.accent_foreground = theme.foreground;
        theme.button = theme.background;
        theme.button_foreground = theme.foreground;
        theme.button_hover = theme.sidebar_accent;
        theme.button_active = rgb(if dark { 0x404047 } else { 0xe7e6e4 }).into();
        theme.secondary = theme.muted;
        theme.secondary_foreground = theme.foreground;
        theme.secondary_hover = theme.button_hover;
        theme.secondary_active = theme.button_active;
        theme.input = rgb(if dark { 0x4e4e56 } else { 0xd2d1cf }).into();
        theme.switch = theme.button_active;
        theme.switch_thumb = rgb(0xffffff).into();
        theme.caret = theme.primary;
        theme.selection = theme.primary.opacity(0.2);
        theme.popover = theme.background;
        theme.popover_foreground = theme.foreground;
        theme.group_box = theme.background;
        theme.group_box_foreground = theme.foreground;
        theme.colors.list = theme.background;
        theme.list_hover = theme.sidebar_accent;
        theme.list_active = theme.sidebar_accent;
        theme.list_active_border = theme.transparent;
        theme.tab = theme.transparent;
        theme.tab_foreground = theme.muted_foreground;
        theme.tab_active = theme.background;
        theme.tab_active_foreground = theme.foreground;
        theme.tab_bar = theme.transparent;
        theme.tab_bar_segmented = theme.muted;
        theme.font_size = px(14.);
    });
}
