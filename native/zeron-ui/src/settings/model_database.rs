//! Official model data uses the same validated, atomic store as the old desktop.
use gpui_kit::{prelude::*, *};
use gpui_kit::component::{ActiveTheme, Disableable, Sizable, StyledExt, button::Button};

pub struct ModelDatabaseUpdate {
    root: std::path::PathBuf,
    version: Option<String>,
    busy: bool,
    error: Option<String>,
    task: Option<Task<()>>,
}
impl ModelDatabaseUpdate {
    pub(crate) fn is_busy(&self) -> bool { self.busy }
    pub fn new(root: std::path::PathBuf, cx: &mut Context<Self>) -> Self {
        let mut this = Self { root, version: None, busy: false, error: None, task: None };
        this.load(false, cx);
        this
    }
    fn load(&mut self, update: bool, cx: &mut Context<Self>) {
        if self.busy { return; }
        self.busy = true;
        self.error = None;
        let root = self.root.clone();
        self.task = Some(cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(async move {
                if update {
                    crate::chat_runtime::runtime()?.block_on(kivio_model_runtime::model_database::update_official(root))
                } else { Ok(kivio_model_runtime::model_database::current_version()) }
            }).await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                match result { Ok(version) => this.version = Some(version), Err(error) => this.error = Some(error) }
                cx.notify();
            });
        }));
        cx.notify();
    }
}
impl Render for ModelDatabaseUpdate {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div().v_flex().gap_2()
            .child(div().flex().items_center().justify_between().gap_4()
                .child(div().v_flex().gap_1().child(crate::i18n::text("模型数据库"))
                    .child(div().text_sm().text_color(cx.theme().muted_foreground)
                        .child(self.version.clone().unwrap_or_else(|| "—".into()))))
                .child(Button::new("update-model-database").small()
                    .label(crate::i18n::text(if self.busy { "读取中…" } else { "检查更新" }))
                    .disabled(self.busy).on_click(cx.listener(|this, _, _, cx| this.load(true, cx)))))
            .when_some(self.error.clone(), |view, error| view.child(div().text_sm().text_color(cx.theme().danger).child(error)))
    }
}
