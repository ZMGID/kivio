//! UI language is persisted in settingsLanguage. Kit's locale is the single
//! runtime locale; model response and translation languages are independent.
use std::{collections::HashMap, sync::OnceLock};
use gpui_kit::{App, SharedString};

pub fn init(language: Option<&str>) {
    if language == Some("en") { let _ = english(); }
    gpui_kit::component::set_locale(if language == Some("en") { "en" } else { "zh-CN" });
}
pub fn apply(language: Option<&str>, cx: &mut App) {
    init(language);
    cx.set_menus(crate::app_menus::app_menus());
    cx.refresh_windows();
}
fn english() -> &'static HashMap<String, SharedString> {
    static ENGLISH: OnceLock<HashMap<String, SharedString>> = OnceLock::new();
    ENGLISH.get_or_init(|| {
        let catalog: HashMap<String, String> = serde_json::from_str(include_str!("../assets/locales/en.json")).expect("valid UI catalog");
        catalog.into_iter().map(|(key, value)| (key, value.into())).collect()
    })
}
pub fn text(source: &str) -> &str {
    if !is_english() { return source; }
    english().get(source).map(|value| value.as_ref()).unwrap_or(source)
}
pub fn display(source: impl Into<SharedString>) -> SharedString {
    let source = source.into();
    if is_english() { english().get(source.as_ref()).cloned().unwrap_or(source) } else { source }
}

pub fn is_english() -> bool { gpui_kit::component::locale().starts_with("en") }

/// Keep Rust's format checking, numeric precision and named argument capture.
#[macro_export]
macro_rules! ui_format {
    ($zh:literal, $en:literal $(, $($args:tt)*)?) => {
        if $crate::i18n::is_english() { format!($en $(, $($args)*)?) }
        else { format!($zh $(, $($args)*)?) }
    };
}
