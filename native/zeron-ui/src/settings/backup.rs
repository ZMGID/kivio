//! Native host of the original staged backup transaction.
use gpui_kit::component::{
    ActiveTheme, Disableable, Sizable, StyledExt, WindowExt, button::Button, switch::Switch,
};
use gpui_kit::{prelude::*, *};
use kivio_model_runtime::{backup, native_backup};
use std::path::PathBuf;

pub struct BackupPage {
    root: PathBuf,
    webdav: Entity<super::backup_webdav::WebDavPage>,
    busy: bool,
    pending: bool,
    trusted: bool,
    status: Option<Result<String, String>>,
    task: Option<Task<()>>,
}
impl BackupPage {
    pub(crate) fn is_busy(&self, cx: &App) -> bool {
        self.busy || self.webdav.read(cx).is_busy()
    }

    pub(crate) fn has_unsaved_changes(&self, cx: &App) -> bool {
        self.webdav.read(cx).has_unsaved_changes(cx)
    }

    pub fn new(root: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let webdav = cx.new(|cx| super::backup_webdav::WebDavPage::new(root.clone(), window, cx));
        let mut page = Self {
            root,
            webdav,
            busy: false,
            pending: false,
            trusted: false,
            status: None,
            task: None,
        };
        let root = page.root.clone();
        page.task = Some(cx.spawn(async move |this, cx| {
            let pending = cx
                .background_spawn(async move { native_backup::pending(&root) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.pending = pending;
                cx.notify();
            });
        }));
        page
    }
    fn export(&mut self, configuration: bool, cx: &mut Context<Self>) {
        if self.busy || self.pending {
            return;
        }
        let prompt = cx.prompt_for_new_path(
            &self.root,
            Some(if configuration {
                "Kivio-settings.json"
            } else {
                "Kivio-backup.kivio"
            }),
        );
        let root = self.root.clone();
        self.busy = true;
        self.status = None;
        self.task = Some(cx.spawn(async move |this, cx| {
            let result = match prompt.await {
                Ok(Ok(Some(path))) => Some(
                    cx.background_spawn(async move {
                        let skipped = if configuration {
                            native_backup::export_configuration(&root, &path)?;
                            Vec::new()
                        } else {
                            native_backup::export_full(&root, &path)?
                        };
                        Ok(crate::ui_format!("备份已保存：{}{}", "Backup saved: {}{}",
                            path.display(),
                            if skipped.is_empty() {
                                String::new()
                            } else {
                                crate::ui_format!("；跳过 {} 个越界或无效链接", "; skipped {} invalid or out-of-bounds links", skipped.len())
                            }
                        ))
                    })
                    .await,
                ),
                Ok(Ok(None)) => None,
                Ok(Err(error)) => Some(Err(error.to_string())),
                Err(error) => Some(Err(error.to_string())),
            };
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                this.status = result;
                cx.notify();
            });
        }));
        cx.notify();
    }
    fn choose_restore(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || self.pending {
            return;
        }
        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("选择 Kivio 配置或完全备份".into()),
        });
        let handle = window.window_handle();
        self.busy = true;
        self.task = Some(cx.spawn(async move |this, cx| {
            let selected = prompt.await;
            let _ = cx.update_window(handle, |_, window, cx| {
                let _ = this.update(cx, |page, cx| {
                    page.busy = false;
                    match selected {
                        Ok(Ok(Some(paths))) => if let Some(path) = paths.into_iter().next() {
                            let owner = cx.weak_entity();
                            window.open_dialog(cx, move |dialog, _, _| {
                                let owner = owner.clone(); let path = path.clone();
                                dialog.title(crate::i18n::text("恢复备份"))
                                    .child(crate::ui_format!("校验并准备恢复“{}”？重启后替换当前数据，原数据会保留为回滚副本。是否保留脚本与任务的启用状态，取决于“信任备份”选项。", "Validate and stage “{}” for restoration? Restarting will replace current data and retain the original for rollback. The backup trust option controls whether scripts and tasks remain enabled.", path.file_name().unwrap_or_default().to_string_lossy()))
                                    .footer(div().flex().justify_end().gap_2()
                                        .child(Button::new("cancel-restore").small().label(crate::i18n::text("取消")).on_click(|_, window, cx| window.close_dialog(cx)))
                                        .child(Button::new("stage-restore").small().label(crate::i18n::text("准备恢复")).on_click(move |_, window, cx| {
                                            let _ = owner.update(cx, |page, cx| page.stage(path.clone(), cx));
                                            window.close_dialog(cx);
                                        })))
                            });
                        },
                        Ok(Ok(None)) => {},
                        Ok(Err(e)) => page.status = Some(Err(e.to_string())),
                        Err(e) => page.status = Some(Err(e.to_string())),
                    }
                    cx.notify();
                });
            });
        }));
        cx.notify();
    }
    fn stage(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let root = self.root.clone();
        let trusted = self.trusted;
        self.busy = true;
        self.status = None;
        self.task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    if path.extension().is_some_and(|ext| ext == "json") {
                        native_backup::stage_configuration(&root, &path, trusted)?;
                    } else {
                        native_backup::stage_full(&root, &path, trusted)?;
                    }
                    Ok("备份已校验并准备完成。重启后应用恢复。".to_owned())
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.pending = result.is_ok();
                this.busy = false;
                this.status = Some(result);
                cx.notify();
            });
        }));
        cx.notify();
    }
}
impl Render for BackupPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = crate::theme::Theme::of(cx).for_settings_surface();
        let disabled = self.busy || self.pending;
        let mut body = div().v_flex().gap_8().child(crate::kivio_settings::card("本地备份", vec![
            crate::kivio_settings::row("配置备份", Button::new("export-configuration").small().disabled(disabled).label(crate::i18n::text("导出配置")).on_click(cx.listener(|this, _, _, cx| this.export(true, cx))).into_any_element(), &theme),
            crate::kivio_settings::row("完全备份", div().child(Button::new("export-full").small().disabled(disabled).label(crate::i18n::text("导出完全备份")).on_click(cx.listener(|this, _, _, cx| this.export(false, cx)))).into_any_element(), &theme),
            crate::kivio_settings::row("从文件恢复", div().child(Button::new("import-full").small().disabled(disabled).label(crate::i18n::text("导入备份")).on_click(cx.listener(|this, _, window, cx| this.choose_restore(window, cx)))).into_any_element(), &theme),
            crate::kivio_settings::row("信任备份中的脚本与任务", Switch::new("trust-local-backup").accessibility_label(crate::i18n::text("信任备份中的脚本与任务")).checked(self.trusted).disabled(disabled).on_click(cx.listener(|this, checked: &bool, _, cx| { this.trusted = *checked; cx.notify(); })).into_any_element(), &theme),
        ], &theme)).child(div().text_sm().text_color(cx.theme().muted_foreground).child(crate::i18n::text("完全备份包含配置、对话、附件和项目文件。备份含密钥；导出前请停止正在写入数据的任务。")));
        if self.pending {
            body = body.child(
                div()
                    .flex()
                    .gap_3()
                    .items_center()
                    .child(crate::i18n::text("恢复已就绪，重启前当前数据保持不变。"))
                    .child(
                        Button::new("restart-restore")
                            .small()
                            .label(crate::i18n::text("重启完成恢复"))
                            .on_click(|_, _, cx| crate::app_menus::request_restart(cx)),
                    ),
            );
        }
        if self.busy {
            body = body.child(crate::i18n::text("正在处理备份…"));
        }
        if let Some(status) = &self.status {
            body = body.child(
                div()
                    .text_sm()
                    .text_color(if status.is_err() {
                        cx.theme().danger
                    } else {
                        cx.theme().muted_foreground
                    })
                    .child(match status {
                        Ok(text) | Err(text) => text.clone(),
                    }),
            );
        }
        crate::kivio_settings::page_frame(
            "备份与同步",
            "完整校验、暂存恢复与重启提交。",
            body.child(self.webdav.clone()).into_any_element(),
            cx,
        )
    }
}

/// No engine or settings writer starts while a restore transaction is incomplete.
pub(crate) fn show_startup_error(root: PathBuf, error: String) {
    struct Recovery {
        root: PathBuf,
        error: String,
    }
    impl Render for Recovery {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let root = self.root.clone();
            div().size_full().v_flex().p_6().gap_4()
                .bg(cx.theme().background).text_color(cx.theme().foreground)
                .child(div().text_lg().child(crate::i18n::text("备份恢复尚未完成")))
                .child(self.error.clone())
                .child(crate::i18n::text("数据和恢复事务已保留。处理错误后重新启动，应用会继续恢复；当前不会启动后台任务。"))
                .child(div().flex().gap_2()
                    .child(Button::new("recovery-folder").small().label(crate::i18n::text("打开数据位置")).on_click(move |_, _, cx| cx.reveal_path(&root)))
                    .child(Button::new("retry-recovery").small().label(crate::i18n::text("重新启动")).on_click(|_, _, cx| crate::app_menus::request_restart(cx)))
                    .child(Button::new("quit-recovery").small().label(crate::i18n::text("退出")).on_click(|_, _, cx| cx.quit())))
        }
    }
    gpui_kit::platform::application().run(move |cx| {
        gpui_kit::init(cx);
        if let Err(error) = gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| {
            cx.new(|_| Recovery { root, error })
        }) {
            eprintln!("无法显示恢复错误：{error}");
            cx.quit();
        }
        cx.activate(true);
    });
}
