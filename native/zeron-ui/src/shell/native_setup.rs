//! Kivio's setup path reuses the retained settings editors and their save owners.
use super::*;
use gpui_kit::component::{Disableable, Sizable, StyledExt, button::{Button, ButtonVariants}};
use kivio_model_runtime::native_preferences::{self, Change, FieldEdit};

pub(super) struct NativeSetup {
    step: usize,
    busy: bool,
    error: Option<String>,
    task: Option<Task<()>>,
}
impl Shell {
    pub(crate) fn start_native_setup(&mut self, cx: &mut Context<Self>) {
        if self.native_setup.is_some() { return; }
        self.native_setup = Some(NativeSetup { step: 0, busy: false, error: None, task: None });
        self.open_settings(SettingsSection::General, cx);
        self.save_native_setup("pending", cx);
    }
    fn save_native_setup(&mut self, status: &'static str, cx: &mut Context<Self>) {
        let Some(setup) = self.native_setup.as_mut() else { return; };
        if setup.busy { return; }
        setup.busy = true;
        setup.error = None;
        let directory = self.data_dir.clone();
        let task = cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(async move {
                let settings = native_preferences::load(&directory)?;
                native_preferences::save(&directory, Change::Fields(vec![FieldEdit {
                    path: "/onboardingStatus".into(), before: settings.onboarding_status.into(), after: status.into(),
                }]))
            }).await;
            let _ = this.update(cx, |this, cx| {
                let Some(setup) = this.native_setup.as_mut() else { return; };
                setup.busy = false;
                setup.task = None;
                match result {
                    Ok(_) if status != "pending" => {
                        this.native_setup = None;
                        this.open_settings(SettingsSection::General, cx);
                    }
                    Ok(_) => {}
                    Err(error) => setup.error = Some(error),
                }
                cx.notify();
            });
        });
        self.native_setup.as_mut().unwrap().task = Some(task);
        cx.notify();
    }
    fn native_setup_editor_busy_or_dirty(&self, step: usize, cx: &App) -> bool {
        (matches!(step, 1 | 4) && self.provider_page.as_ref().is_some_and(|page| page.read(cx).is_busy() || page.read(cx).has_unsaved_changes(cx)))
            || (matches!(step, 2 | 4) && self.search_page.as_ref().is_some_and(|page| page.read(cx).is_busy() || page.read(cx).has_unsaved_changes(cx)))
            || (matches!(step, 3 | 4) && self.kivio_settings_page.as_ref().is_some_and(|page| page.read(cx).is_busy(cx) || page.read(cx).has_unsaved_changes(cx)))
    }
    pub(super) fn render_native_setup(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let setup = self.native_setup.as_ref().unwrap();
        let step = setup.step;
        let busy = setup.busy;
        let error = setup.error.clone();
        let blocked = busy || self.native_setup_editor_busy_or_dirty(step, cx);
        let theme = Theme::of(cx).for_settings_surface();
        settings::widgets::set_sidebar_width(0., cx);
        let body = match step {
            1 => self.settings_outlet(SettingsSection::Harnesses, window, cx),
            2 => self.settings_outlet(SettingsSection::Kivio(crate::kivio_settings::Section::Search), window, cx),
            3 => self.settings_outlet(SettingsSection::Shortcuts, window, cx),
            _ => {
                let (title, subtitle) = if step == 0 {
                    ("欢迎使用 Kivio", "配置模型、网络搜索和快捷键。所有更改保存到本机，之后可随时在设置中调整。")
                } else {
                    ("设置完成", "模型、网络搜索和快捷键已按你的选择保存。现在可以返回 Kivio。")
                };
                crate::kivio_settings::page_frame(title, subtitle,
                    div().v_flex().gap_4()
                        .child(crate::i18n::text("模型：选择供应商、凭据和默认模型。"))
                        .child(crate::i18n::text("网络搜索：选择搜索服务并测试连接。"))
                        .child(crate::i18n::text("快捷键：配置截图、翻译和窗口操作。"))
                        .into_any_element(), cx)
            }
        };
        let mut footer = div().flex().items_center().justify_between().gap_3();
        footer = footer.child(Button::new("setup-skip").small().label(crate::i18n::text("稍后设置")).disabled(busy)
            .on_click(cx.listener(|this, _, _, cx| this.save_native_setup("skipped", cx))));
        let mut navigation = div().flex().gap_2();
        if step > 0 {
            navigation = navigation.child(Button::new("setup-back").small().label(crate::i18n::text("上一步")).disabled(busy)
                .on_click(cx.listener(|this, _, _, cx| {
                    if let Some(setup) = this.native_setup.as_mut() { setup.step = setup.step.saturating_sub(1); }
                    cx.notify();
                })));
        }
        navigation = navigation.child(Button::new("setup-next").small().primary()
            .label(crate::i18n::text(if step == 4 { "完成" } else { "继续" })).disabled(blocked)
            .on_click(cx.listener(move |this, _, _, cx| {
                if step == 4 { this.save_native_setup("completed", cx); }
                else if let Some(setup) = this.native_setup.as_mut() { setup.step += 1; cx.notify(); }
            })));
        footer = footer.child(navigation);
        div().id("native-setup").size_full().v_flex().bg(theme.bg)
            .pt(px(Theme::TITLEBAR_HEIGHT))
            .child(div().px_6().py_3().child(crate::onboarding::progress(&theme, step + 1, 5)))
            .child(div().flex_1().min_h_0().child(body))
            .child(div().px_6().py_4().border_t_1().border_color(theme.border)
                .when_some(error, |view, error| view.child(div().text_sm().text_color(theme.danger).child(error)))
                .when(blocked && !busy, |view| view.child(div().text_sm().text_color(theme.text_muted)
                    .child(crate::i18n::text("请先保存当前修改，再继续。"))))
                .child(footer))
            .into_any_element()
    }
}
