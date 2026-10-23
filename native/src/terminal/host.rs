//! Connect the adapted Zeron view to Kivio's existing PTY service.
use super::{TerminalEvent, TerminalView};
use gpui_kit::component::{ActiveTheme, StyledExt};
use gpui_kit::{prelude::FluentBuilder, *};
use kivio_model_runtime::terminal::{self, TerminalService};
use std::{path::PathBuf, sync::Arc};

struct Session {
    service: Arc<TerminalService>,
    id: String,
}
impl Drop for Session {
    fn drop(&mut self) {
        self.service.close(&self.id);
    }
}

pub struct TerminalHost {
    view: Entity<TerminalView>,
    directory: PathBuf,
    status: SharedString,
    commands: Option<std::sync::mpsc::Sender<TerminalEvent>>,
    session: Option<Session>,
    _task: Option<Task<()>>,
    _subscription: Subscription,
}
impl TerminalHost {
    pub fn new(directory: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let view = cx.new(TerminalView::new);
        view.focus_handle(cx).focus(window, cx);
        let subscription = cx.subscribe(&view, |this: &mut Self, _, event, cx| {
            if let Some(commands) = &this.commands {
                if commands.send(event.clone()).is_err() {
                    this.status = "终端输入通道已关闭".into();
                    cx.notify();
                }
            }
        });
        let (events, output) = async_channel::bounded(128);
        let service = Arc::new(TerminalService::new(move |event| {
            events.send_blocking(event).map_err(|e| e.to_string())
        }));
        let workdir = directory.clone();
        let work = cx.background_spawn(async move {
            let result =
                terminal::create_session(service.clone(), &workdir.to_string_lossy(), 80, 24)?;
            Ok::<_, String>(Session {
                service,
                id: result.session_id,
            })
        });
        let task = cx.spawn(async move |owner, cx| {
            match work.await {
                Err(error) => {
                    let _ = owner.update(cx, |this, cx| {
                        this.status = error.into();
                        cx.notify();
                    });
                }
                Ok(session) => {
                    let (commands, input) = std::sync::mpsc::channel();
                    let service = session.service.clone();
                    let id = session.id.clone();
                    // Serialized blocking writes never run on the GPUI thread.
                    let worker = std::thread::Builder::new()
                        .name("native-terminal-input".into())
                        .spawn(move || {
                            while let Ok(command) = input.recv() {
                                let result = match command {
                                    TerminalEvent::Write(bytes) => service.write(&id, &bytes),
                                    TerminalEvent::Resize { cols, rows } => {
                                        service.resize(&id, cols, rows)
                                    }
                                };
                                if result.is_err() {
                                    break;
                                }
                            }
                        });
                    if let Err(error) = worker {
                        let _ = owner.update(cx, |this, cx| {
                            this.status = error.to_string().into();
                            cx.notify();
                        });
                        return;
                    }
                    if owner
                        .update(cx, |this, cx| {
                            this.commands = Some(commands);
                            this.session = Some(session);
                            this.status = "".into();
                            this.view.update(cx, |view, cx| view.resend_size(cx));
                            cx.notify();
                        })
                        .is_err()
                    {
                        return;
                    }
                    while let Ok(event) = output.recv().await {
                        if owner
                            .update(cx, |this, cx| {
                                match event {
                                    terminal::TerminalEvent::Output { data, .. } => {
                                        this.view.update(cx, |view, cx| view.feed_output(&data, cx))
                                    }
                                    terminal::TerminalEvent::Exit { code, .. } => {
                                        this.commands = None;
                                        this.session = None;
                                        this.status = format!("进程已退出 · {code}").into();
                                    }
                                }
                                cx.notify();
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                }
            }
        });
        Self {
            view,
            directory,
            status: "正在启动终端…".into(),
            commands: None,
            session: None,
            _task: Some(task),
            _subscription: subscription,
        }
    }
}
impl Render for TerminalHost {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .v_flex()
            .size_full()
            .min_h_0()
            .child(
                div()
                    .px_3()
                    .py_1()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .overflow_hidden()
                    .text_ellipsis()
                    .child(self.directory.to_string_lossy().into_owned()),
            )
            .when(!self.status.is_empty(), |body| {
                body.child(div().px_3().text_sm().child(self.status.clone()))
            })
            .child(div().flex_1().min_h_0().child(self.view.clone()))
    }
}
