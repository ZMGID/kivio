use gpui_kit::component::{
    ActiveTheme, Disableable, Sizable, StyledExt, WindowExt,
    button::Button,
    input::{Input, InputState},
    switch::Switch,
};
use gpui_kit::{prelude::*, *};
use kivio_model_runtime::{
    native_backup,
    webdav::{self, WebDavBackup, WebDavConfig, WebDavConfigInput},
};
use std::path::PathBuf;

#[derive(Clone)]
enum Action {
    Save,
    Test,
    List,
    Upload(bool),
    Restore(String),
    Delete(String),
}
pub struct WebDavPage {
    root: PathBuf,
    url: Entity<InputState>,
    username: Entity<InputState>,
    password: Entity<InputState>,
    directory: Entity<InputState>,
    confirmed: Option<WebDavConfig>,
    has_password: bool,
    clear_password: bool,
    trusted: bool,
    busy: bool,
    pending: bool,
    files: Vec<WebDavBackup>,
    status: Option<Result<String, String>>,
    task: Option<Task<()>>,
    _inputs: Vec<Subscription>,
}
impl WebDavPage {
    pub(crate) fn is_busy(&self) -> bool {
        self.busy
    }

    pub(crate) fn has_unsaved_changes(&self, cx: &App) -> bool {
        let (url, username, directory) = self
            .confirmed
            .as_ref()
            .map(|config| {
                (
                    config.url.as_str(),
                    config.username.as_str(),
                    config.directory.as_str(),
                )
            })
            .unwrap_or(("", "", "Kivio"));
        self.url.read(cx).value().as_ref() != url
            || self.username.read(cx).value().as_ref() != username
            || self.directory.read(cx).value().as_ref() != directory
            || !self.password.read(cx).value().is_empty()
            || self.clear_password
    }

    fn transfer_ready(&self, cx: &App) -> bool {
        self.confirmed
            .as_ref()
            .is_some_and(|config| !config.url.trim().is_empty())
            && !self.has_unsaved_changes(cx)
    }

    fn discard_changes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || self.pending {
            return;
        }
        let config = self.confirmed.take().unwrap_or(WebDavConfig {
            url: String::new(),
            username: String::new(),
            directory: "Kivio".into(),
            has_password: false,
        });
        self.accept_configuration(config, window, cx);
        self.status = None;
        cx.notify();
    }

    fn accept_configuration(
        &mut self,
        config: WebDavConfig,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.url.update(cx, |input, cx| {
            input.set_value(config.url.clone(), window, cx)
        });
        self.username.update(cx, |input, cx| {
            input.set_value(config.username.clone(), window, cx)
        });
        self.directory.update(cx, |input, cx| {
            input.set_value(config.directory.clone(), window, cx)
        });
        self.has_password = config.has_password;
        self.clear_password = false;
        self.password
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.confirmed = Some(config);
    }

    pub fn new(root: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut page = Self {
            root: root.clone(),
            url: cx.new(|cx| InputState::new(window, cx).placeholder("https://example.com/dav")),
            username: cx.new(|cx| InputState::new(window, cx)),
            password: cx.new(|cx| InputState::new(window, cx).masked(true)),
            directory: cx.new(|cx| InputState::new(window, cx).default_value("Kivio")),
            confirmed: None,
            has_password: false,
            clear_password: false,
            trusted: false,
            busy: true,
            pending: false,
            files: vec![],
            status: None,
            task: None,
            _inputs: vec![],
        };
        page._inputs = [&page.url, &page.username, &page.password, &page.directory]
            .into_iter()
            .map(|input| cx.observe(input, |_, _, cx| cx.notify()))
            .collect();
        let handle = window.window_handle();
        page.task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move { webdav::configuration(&root) })
                .await;
            let _ = cx.update_window(handle, |_, window, cx| {
                let _ = this.update(cx, |this, cx| {
                    this.busy = false;
                    match result {
                        Ok(config) => this.accept_configuration(config, window, cx),
                        Err(error) => this.status = Some(Err(error)),
                    }
                    cx.notify();
                });
            });
        }));
        page
    }
    fn run(&mut self, action: Action, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || self.pending {
            return;
        }
        if !matches!(&action, Action::Save | Action::Test) && !self.transfer_ready(cx) {
            self.status = Some(Err("请先保存或放弃连接修改，再操作远程备份。".into()));
            cx.notify();
            return;
        }
        let root = self.root.clone();
        let handle = window.window_handle();
        let password = self.password.read(cx).value().to_string();
        let trusted = self.trusted;
        let input = WebDavConfigInput {
            url: self.url.read(cx).value().to_string(),
            username: self.username.read(cx).value().to_string(),
            directory: self.directory.read(cx).value().to_string(),
            password: if self.clear_password {
                Some(String::new())
            } else {
                (!password.is_empty()).then_some(password)
            },
        };
        self.busy = true;
        self.status = None;
        self.task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    crate::chat_runtime::runtime()?.block_on(async move {
                        let mut pending = false;
                        let message = match action {
                            Action::Save => {
                                let config = webdav::save(&root, input)?;
                                return Ok((
                                    "WebDAV 配置已保存".to_owned(),
                                    None,
                                    false,
                                    Some(config),
                                ));
                            }
                            Action::Test => {
                                webdav::test(&root, input).await?;
                                return Ok((
                                    "连接、写入、读取和清理验证通过".to_owned(),
                                    None,
                                    false,
                                    None,
                                ));
                            }
                            Action::List => {
                                let files = webdav::list(&root).await?;
                                return Ok(("远程备份已刷新".to_owned(), Some(files), false, None));
                            }
                            Action::Upload(configuration) => {
                                let temp = tempfile::tempdir().map_err(|e| e.to_string())?;
                                let archive = temp.path().join("backup.kivio");
                                if configuration {
                                    native_backup::export_configuration(&root, &archive)?;
                                } else {
                                    native_backup::export_full(&root, &archive)?;
                                }
                                webdav::upload(
                                    &root,
                                    &archive,
                                    if configuration {
                                        webdav::WebDavBackupKind::Configuration
                                    } else {
                                        webdav::WebDavBackupKind::Full
                                    },
                                )
                                .await?;
                                "备份已上传"
                            }
                            Action::Restore(name) => {
                                let temp = tempfile::tempdir().map_err(|e| e.to_string())?;
                                let archive = temp.path().join("backup.kivio");
                                webdav::download(&root, &name, &archive).await?;
                                if name.ends_with(".json") {
                                    native_backup::stage_configuration(&root, &archive, trusted)?;
                                } else {
                                    native_backup::stage_full(&root, &archive, trusted)?;
                                }
                                pending = true;
                                "远程备份已校验，重启后完成恢复"
                            }
                            Action::Delete(name) => {
                                webdav::delete(&root, &name).await?;
                                "远程备份已删除"
                            }
                        };
                        // A successful mutation is not reported as a failed operation merely
                        // because refreshing the remote listing encounters a later network error.
                        if pending {
                            return Ok((message.to_owned(), None, true, None));
                        }
                        let files = webdav::list(&root).await;
                        match files {
                            Ok(files) => Ok((message.to_owned(), Some(files), pending, None)),
                            Err(error) => Ok((
                                crate::ui_format!("{message}；列表刷新失败：{error}", "{message}; failed to refresh the list: {error}"),
                                None,
                                pending,
                                None,
                            )),
                        }
                    })
                })
                .await;
            let _ = cx.update_window(handle, |_, window, cx| {
                this.update(cx, |this, cx| {
                    this.busy = false;
                    this.status = Some(match result {
                        Ok((message, files, pending, config)) => {
                            if let Some(config) = config {
                                this.files.clear();
                                this.accept_configuration(config, window, cx);
                            }
                            if let Some(files) = files {
                                this.files = files;
                            }
                            this.pending = pending;
                            Ok(message)
                        }
                        Err(error) => Err(error),
                    });
                    cx.notify();
                })
            });
        }));
        cx.notify();
    }
    fn confirm(
        &mut self,
        action: Action,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let owner = cx.weak_entity();
        let deleting = matches!(action, Action::Delete(_));
        window.open_dialog(cx, move |dialog, _, _| {
            let owner = owner.clone();
            let action = action.clone();
            dialog
                .title(if deleting {
                    "删除远程备份"
                } else {
                    "恢复远程备份"
                })
                .child(format!(
                    "{}“{}”？{}",
                    if deleting {
                        "删除"
                    } else {
                        "下载并恢复"
                    },
                    name,
                    if deleting {
                        "此操作会永久删除服务器上的此备份。"
                    } else {
                        "先校验并暂存，重启后生效，原数据保留回滚副本。"
                    }
                ))
                .footer(
                    div()
                        .flex()
                        .justify_end()
                        .gap_2()
                        .child(
                            Button::new("cancel-dav")
                                .small()
                                .label(crate::i18n::text("取消"))
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(
                            Button::new("confirm-dav")
                                .small()
                                .label(if deleting { crate::i18n::text("删除") } else { crate::i18n::text("准备恢复") })
                                .on_click(move |_, window, cx| {
                                    let _ = owner.update(cx, |this, cx| {
                                        this.run(action.clone(), window, cx)
                                    });
                                    window.close_dialog(cx);
                                }),
                        ),
                )
        });
    }
}
impl Render for WebDavPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let disabled = self.busy || self.pending;
        let dirty = self.has_unsaved_changes(cx);
        let transfer_disabled = disabled || !self.transfer_ready(cx);
        let mut body = div()
            .v_flex()
            .gap_3()
            .p_4()
            .rounded(px(12.))
            .bg(cx.theme().group_box)
            .child("WebDAV")
            .child(div().text_sm().child(crate::i18n::text("服务器地址")))
            .child(
                Input::new(&self.url)
                    .small()
                    .aria_label("WebDAV 地址")
                    .disabled(disabled),
            )
            .child(div().text_sm().child(crate::i18n::text("用户名")))
            .child(
                Input::new(&self.username)
                    .small()
                    .aria_label("WebDAV 用户名")
                    .disabled(disabled),
            )
            .child(div().text_sm().child(crate::i18n::text("密码")))
            .child(
                Input::new(&self.password)
                    .small()
                    .aria_label("WebDAV 密码，留空保留已存密码")
                    .disabled(disabled),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(if self.has_password { crate::i18n::text("已保存密码；留空保留。") } else { crate::i18n::text("输入 WebDAV 密码。") }),
            )
            .child(
                Switch::new("dav-clear-password")
                    .label(crate::i18n::text("清除已保存的密码"))
                    .checked(self.clear_password)
                    .disabled(disabled)
                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                        this.clear_password = *checked;
                        cx.notify();
                    })),
            )
            .child(div().text_sm().child(crate::i18n::text("远程目录")))
            .child(
                Input::new(&self.directory)
                    .small()
                    .aria_label("WebDAV 远程目录")
                    .disabled(disabled),
            );
        let mut actions = div().flex().flex_wrap().gap_2();
        for (id, label, action) in [
            ("save-dav", "保存配置", Action::Save),
            ("test-dav", "测试连接", Action::Test),
            ("refresh-dav", "刷新备份", Action::List),
            ("upload-config-dav", "上传配置", Action::Upload(true)),
            ("upload-dav", "上传完全备份", Action::Upload(false)),
        ] {
            let action_disabled = if matches!(&action, Action::Save | Action::Test) {
                disabled
            } else {
                transfer_disabled
            };
            actions =
                actions.child(
                    Button::new(id)
                        .small()
                        .label(crate::i18n::display(label))
                        .disabled(action_disabled)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.run(action.clone(), window, cx)
                        })),
                );
        }
        if dirty {
            actions = actions.child(
                Button::new("discard-dav")
                    .small()
                    .label(crate::i18n::text("放弃修改"))
                    .disabled(disabled)
                    .on_click(cx.listener(|this, _, window, cx| this.discard_changes(window, cx))),
            );
            body = body.child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(crate::i18n::text("连接有未保存的修改。先保存或放弃修改，再上传、读取或删除远程备份。")),
            );
        }
        body = body.child(actions).child(
            Switch::new("dav-trust-restore")
                .label(crate::i18n::text("信任远程备份中的脚本与任务"))
                .checked(self.trusted)
                .disabled(disabled)
                .on_click(cx.listener(|this, checked: &bool, _, cx| {
                    this.trusted = *checked;
                    cx.notify();
                })),
        );
        for file in &self.files {
            let name = file.name.clone();
            let restore = name.clone();
            let delete = name.clone();
            body = body.child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .child(
                        div().flex_1().min_w_0().child(name.clone()).child(
                            div()
                                .text_sm()
                                .child(format!("{} · {} bytes", file.modified, file.size)),
                        ),
                    )
                    .child(
                        Button::new(SharedString::from(format!("restore-{name}")))
                            .small()
                            .label(crate::i18n::text("恢复"))
                            .disabled(transfer_disabled)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.confirm(
                                    Action::Restore(restore.clone()),
                                    restore.clone(),
                                    window,
                                    cx,
                                )
                            })),
                    )
                    .child(
                        Button::new(SharedString::from(format!("delete-{name}")))
                            .small()
                            .label(crate::i18n::text("删除"))
                            .disabled(transfer_disabled)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.confirm(
                                    Action::Delete(delete.clone()),
                                    delete.clone(),
                                    window,
                                    cx,
                                )
                            })),
                    ),
            );
        }
        if self.busy {
            body = body.child(crate::i18n::text("正在处理 WebDAV…"));
        }
        if let Some(result) = &self.status {
            body = body.child(
                div()
                    .text_sm()
                    .text_color(if result.is_err() {
                        cx.theme().danger
                    } else {
                        cx.theme().muted_foreground
                    })
                    .child(match result {
                        Ok(v) | Err(v) => v.clone(),
                    }),
            );
        }
        if self.pending {
            body = body.child(
                Button::new("restart-dav")
                    .small()
                    .label(crate::i18n::text("重启完成恢复"))
                    .on_click(|_, _, cx| crate::app_menus::request_restart(cx)),
            );
        }
        body
    }
}
