//! Kivio uses the system browser. The imported WebView hosts remain source-only.
#[path = "browser/model.rs"]
pub mod model;
#[path = "browser/profile.rs"]
pub(crate) mod profile;
use gpui_kit::{prelude::*, *};
#[derive(Clone, Debug)]
pub enum BrowserEvent { Changed, NewTab(Option<String>), Close }
#[derive(Clone, Default)]
pub struct BrowserContext;
impl BrowserContext { pub(crate) fn for_profile(_: profile::BrowserProfile) -> Self { Self } }
pub(crate) fn bind_keys(_: &mut App, _: &crate::settings::KeymapConfig) {}
pub struct BrowserSurface { pub page: model::PageState, pub favicon: Option<std::sync::Arc<Image>>, focus: FocusHandle }
impl EventEmitter<BrowserEvent> for BrowserSurface {}
impl Focusable for BrowserSurface { fn focus_handle(&self, _: &App) -> FocusHandle { self.focus.clone() } }
impl BrowserSurface {
    pub fn new(_: BrowserContext, _: bool, _: &mut Window, cx: &mut Context<Self>) -> Self {
        Self { page: model::PageState::default(), favicon: None, focus: cx.focus_handle() }
    }
    pub fn title(&self) -> SharedString { "系统浏览器".into() }
    pub fn set_remote(&mut self, _: bool) {}
    pub fn set_shortcuts(&mut self, _: &crate::settings::KeymapConfig) {}
    pub fn focus_address(&mut self, window: &mut Window, cx: &mut Context<Self>) { window.focus(&self.focus, cx); }
    pub fn set_resize_inset(&mut self, _: Pixels, _: &mut Context<Self>) {}
    pub fn set_right_occlusion(&mut self, _: Pixels, _: &mut Context<Self>) {}
    pub fn set_presentation(&mut self, _: model::Presentation, _: &mut Context<Self>) {}
    pub fn watch_previews(&mut self, _: crate::state::EngineHandle, _: String, _: &mut Context<Self>) {}
    pub fn navigate(&mut self, input: &str, _: &mut Window, cx: &mut Context<Self>) {
        match model::normalize_address(input) {
            Ok(url) => { cx.open_url(&url); self.page.url = Some(url); self.page.error = None; }
            Err(message) => self.page.error = Some(message.into()),
        }
        cx.emit(BrowserEvent::Changed); cx.notify();
    }
    pub fn close(&mut self, cx: &mut Context<Self>) { cx.emit(BrowserEvent::Close); }
}
impl Render for BrowserSurface {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().flex().items_center().justify_center().child(
            self.page.error.clone().unwrap_or_else(|| "网页链接由系统浏览器打开".into()))
    }
}
