//! Built-in browser plugin setup, retained independently of the settings selection.
use gpui_kit::component::{
    ActiveTheme, Disableable, Sizable, StyledExt, button::Button, switch::Switch,
};
use gpui_kit::{prelude::*, *};
use kivio_model_runtime::plugins::{catalog_plugin, native_control};
use std::{collections::VecDeque, path::PathBuf};
#[derive(Clone, Copy)]
enum Operation {
    Detect,
    Install,
    Enable(bool),
}
pub struct EgoControl {
    root: PathBuf,
    busy: bool,
    revision: u64,
    status: Option<native_control::Status>,
    error: Option<String>,
    message: Option<SharedString>,
    logs: VecDeque<SharedString>,
    task: Option<Task<()>>,
}
impl EgoControl {
    pub fn new(root: PathBuf, cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            root,
            busy: false,
            revision: kivio_model_runtime::native_preferences::revision(),
            status: None,
            error: None,
            message: None,
            logs: VecDeque::new(),
            task: None,
        };
        this.run(Operation::Detect, cx);
        this
    }
    pub fn is_busy(&self) -> bool {
        self.busy
    }
    fn run(&mut self, operation: Operation, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.error = None;
        self.message = None;
        if matches!(operation, Operation::Install) {
            self.logs.clear();
        }
        let root = self.root.clone();
        self.task = Some(cx.spawn(async move |this, cx| {
            let (sender, receiver) = async_channel::bounded::<String>(128);
            let worker = cx.background_spawn(async move {
                crate::chat_runtime::runtime()?.block_on(async move {
                    let result = match operation {
                        Operation::Detect => native_control::status(&root).map(|status| (status, None)),
                        Operation::Enable(enabled) => native_control::set_enabled(&root, enabled).await,
                        Operation::Install => native_control::install(&root, &|line| { let _ = sender.try_send(line); }).await.map(|status| {
                            let message = if status.installed { "安装已完成，请启用插件。" } else { "安装命令已结束，尚未检测到 ego-browser。请完成应用首次设置后重新检测。" };
                            (status, Some(message.to_string()))
                        }),
                    };
                    // Refresh even after partial failure, so enabled/installed never stays stale.
                    let fallback = if result.is_err() { native_control::status(&root).ok() } else { None };
                    Ok::<_, String>((result, fallback))
                })
            });
            while let Ok(line) = receiver.recv().await {
                let mut batch = vec![line];
                while batch.len() < 128 { match receiver.try_recv() { Ok(line) => batch.push(line), Err(_) => break } }
                if this.update(cx, |this, cx| {
                    for line in batch { this.logs.push_back(line.chars().take(2000).collect::<String>().into()); }
                    while this.logs.len() > 200 { this.logs.pop_front(); }
                    cx.notify();
                }).is_err() { return; }
            }
            let result = worker.await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                match result {
                    Ok((Ok((status, message)), _)) => {
                        this.status = Some(status); this.message = message.map(Into::into);
                    }
                    Ok((Err(error), fallback)) => { this.status = fallback.or_else(|| this.status.take()); this.error = Some(error); }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }
}
impl Render for EgoControl {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let revision = kivio_model_runtime::native_preferences::revision();
        if !self.busy && revision != self.revision {
            self.revision = revision;
            self.run(Operation::Detect, cx);
        }
        let installed = self.status.as_ref().is_some_and(|status| status.installed);
        let enabled = self.status.as_ref().is_some_and(|status| status.enabled);
        let effective = self.status.as_ref().is_some_and(|status| status.effective);
        let ready = self
            .status
            .as_ref()
            .is_some_and(|status| status.skill_ready);
        let status = if self.busy {
            "正在处理…"
        } else if !installed {
            "未安装"
        } else if !enabled {
            "已安装，未启用"
        } else if !ready {
            "已启用，Skill 尚未就绪"
        } else if !effective {
            "已启用，工具开关已关闭"
        } else {
            "已启用，可用"
        };
        let mut actions = div().flex().items_center().gap_2();
        if installed {
            actions = actions.child(
                Switch::new("ego-enabled")
                    .checked(enabled)
                    .disabled(self.busy)
                    .on_click(cx.listener(|this, enabled, _, cx| {
                        this.run(Operation::Enable(*enabled), cx)
                    })),
            );
            if enabled && !ready {
                actions = actions.child(
                    Button::new("ego-repair")
                        .small()
                        .label(crate::i18n::text("重试 Skill 下载"))
                        .disabled(self.busy)
                        .on_click(
                            cx.listener(|this, _, _, cx| this.run(Operation::Enable(true), cx)),
                        ),
                );
            }
            if enabled && ready && !effective {
                actions = actions.child(
                    Button::new("ego-restore-tools")
                        .small()
                        .label(crate::i18n::text("恢复工具开关"))
                        .disabled(self.busy)
                        .on_click(
                            cx.listener(|this, _, _, cx| this.run(Operation::Enable(true), cx)),
                        ),
                );
            }
        } else if catalog_plugin("ego-lite")
            .and_then(|catalog| catalog.host_install_command())
            .is_some()
        {
            actions = actions.child(
                Button::new("ego-install")
                    .small()
                    .label(crate::i18n::text("安装"))
                    .disabled(self.busy)
                    .on_click(cx.listener(|this, _, _, cx| this.run(Operation::Install, cx))),
            );
        }
        actions = actions.child(
            Button::new("ego-refresh")
                .small()
                .label(crate::i18n::text("重新检测"))
                .disabled(self.busy)
                .on_click(cx.listener(|this, _, _, cx| this.run(Operation::Detect, cx))),
        );
        let mut content = div()
            .v_flex()
            .gap_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_4()
                    .py_3()
                    .child(
                        div()
                            .v_flex()
                            .gap_1()
                            .child("ego lite")
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(crate::i18n::display(status)),
                            )
                            .when_some(
                                self.status
                                    .as_ref()
                                    .and_then(|status| status.version.clone()),
                                |view, version| view.child(div().text_sm().child(version)),
                            ),
                    )
                    .child(actions),
            )
            .child(
                Button::new("ego-homepage")
                    .small()
                    .label(crate::i18n::text("官网与首次设置说明"))
                    .on_click(|_, _, cx| {
                        if let Some(catalog) = catalog_plugin("ego-lite") {
                            cx.open_url(catalog.homepage);
                        }
                    }),
            );
        if let Some(message) = &self.message {
            content = content.child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(crate::i18n::display(message)),
            );
        }
        if let Some(error) = &self.error {
            content = content.child(
                div()
                    .text_sm()
                    .text_color(cx.theme().danger)
                    .child(error.clone()),
            );
        }
        if !self.logs.is_empty() {
            content = content.child(super::widgets::scroll_faded(
                "ego-install-log",
                div()
                    .id("ego-install-log-content")
                    .max_h(px(160.))
                    .overflow_y_scroll()
                    .v_flex()
                    .gap_1()
                    .text_xs()
                    .children(self.logs.iter().cloned().map(|line| div().child(line))),
            ));
        }
        content
    }
}
