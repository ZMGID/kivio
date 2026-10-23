//! Terminal defaults are resolved from Kivio's shared Kit theme. Explicit ANSI
//! RGB and indexed cube colors remain program data, not application chrome.
use gpui_kit::component::ActiveTheme;
use gpui_kit::{App, Hsla, SharedString};

#[derive(Clone, Copy)]
pub enum Appearance {
    Dark,
    Light,
}
#[derive(Clone)]
pub struct Palette {
    pub background: Hsla,
    pub foreground: Hsla,
    pub selection: Hsla,
    pub cursor: Hsla,
    pub ansi: [Hsla; 16],
}
#[derive(Clone)]
pub struct Theme {
    pub appearance: Appearance,
    pub terminal: Palette,
    pub font_terminal: SharedString,
    pub terminal_font_size: f32,
}
impl Theme {
    pub fn of(cx: &App) -> Self {
        let t = cx.theme();
        // ANSI semantic roles follow the application's palette. Bright slots
        // use the corresponding hover tokens rather than a second hardcoded theme.
        let ansi = [
            t.background,
            t.danger,
            t.success,
            t.warning,
            t.primary,
            t.link,
            t.info,
            t.foreground,
            t.muted_foreground,
            t.danger_hover,
            t.success_hover,
            t.warning_hover,
            t.button_primary_hover,
            t.link,
            t.info_hover,
            t.foreground,
        ];
        Self {
            appearance: if t.is_dark() {
                Appearance::Dark
            } else {
                Appearance::Light
            },
            terminal: Palette {
                background: t.background,
                foreground: t.foreground,
                selection: t.selection,
                cursor: t.caret.opacity(0.45),
                ansi,
            },
            font_terminal: t.mono_font_family.clone(),
            terminal_font_size: f32::from(t.mono_font_size),
        }
    }
}
