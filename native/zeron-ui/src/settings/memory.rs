//! The original L1/L2 editors with the shared Rust memory file owner.
use gpui_kit::component::{
    ActiveTheme, Disableable, StyledExt,
    button::{Button, ButtonVariants},
    input::{Textarea, TextareaState},
};
use gpui_kit::{prelude::*, *};
use kivio_model_runtime::memory_files::{self, MemoryLayer};

pub struct MemoryPage {
    directory: std::path::PathBuf,
    editors: [Entity<TextareaState>; 2],
    confirmed: [String; 2],
    busy: bool,
    error: Option<String>,
    task: Option<Task<()>>,
    _editors: Vec<Subscription>,
}
impl MemoryPage {
    pub fn new(directory: std::path::PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            directory,
            editors: std::array::from_fn(|_| cx.new(|cx| TextareaState::new(window, cx).rows(9))),
            confirmed: Default::default(),
            busy: false,
            error: None,
            task: None,
            _editors: vec![],
        };
        this._editors = this
            .editors
            .iter()
            .map(|editor| cx.observe(editor, |_, _, cx| cx.notify()))
            .collect();
        this.load(None, window, cx);
        this
    }
    fn load(&mut self, save: Option<usize>, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.error = None;
        let directory = self.directory.clone();
        let content = save.map(|index| (index, self.editors[index].read(cx).value().to_string()));
        let handle = window.window_handle();
        self.task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let dir = memory_files::memory_dir_at(&directory)?;
                    if let Some((index, content)) = content {
                        let saved = memory_files::save_layer_at(
                            &dir,
                            if index == 0 {
                                MemoryLayer::L1
                            } else {
                                MemoryLayer::L2
                            },
                            &content,
                        )?;
                        Ok::<_, String>(vec![(index, saved.content)])
                    } else {
                        Ok(vec![
                            (
                                0,
                                memory_files::read_layer_at(&dir, MemoryLayer::L1)?.content,
                            ),
                            (
                                1,
                                memory_files::read_layer_at(&dir, MemoryLayer::L2)?.content,
                            ),
                        ])
                    }
                })
                .await;
            let _ = cx.update_window(handle, |_, window, cx| {
                this.update(cx, |this, cx| {
                    this.busy = false;
                    match result {
                        Ok(layers) => {
                            for (index, text) in layers {
                                this.confirmed[index] = text.clone();
                                this.editors[index]
                                    .update(cx, |editor, cx| editor.set_value(text, window, cx));
                            }
                        }
                        Err(error) => this.error = Some(error),
                    }
                    cx.notify();
                })
            });
        }));
        cx.notify();
    }
}
impl Render for MemoryPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut body = div().v_flex().gap_4()
            .child(div().v_flex().gap_2().border_1().border_color(cx.theme().border).rounded_lg().p_4()
                .child(div().font_weight(FontWeight::MEDIUM).child("记忆运行"))
                .child(div().text_sm().text_color(cx.theme().muted_foreground)
                    .child("L1 在线记忆与 L2 长期记忆独立保存。原生 Agent 的自动记忆注入尚未接入。"))
                .child(div().flex().items_center().justify_between().gap_3()
                    .child(div().flex_1().min_w_0().text_sm().child(self.directory.join("chat-memory").display().to_string()))
                    .child(Button::new("memory-reload").label("重载").disabled(self.busy)
                        .on_click(cx.listener(|this, _, window, cx| this.load(None, window, cx))))));
        if let Some(error) = &self.error {
            body = body.child(div().text_color(cx.theme().danger).child(error.clone()));
        }
        for (index, title, hint) in [
            (
                0,
                "L1 在线记忆",
                "保存短小、稳定的当前事实；详细材料放入 L2。",
            ),
            (1, "L2 长期记忆", "保存长期资料，按需检索和读取。"),
        ] {
            let text = self.editors[index].read(cx).value();
            let bytes = text.len();
            let over_limit = index == 0 && bytes > memory_files::L1_MAX_BYTES;
            body = body.child(
                div()
                    .v_flex()
                    .gap_3()
                    .border_1()
                    .border_color(cx.theme().border)
                    .rounded_lg()
                    .p_4()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(div().font_weight(FontWeight::MEDIUM).child(title))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(if over_limit {
                                        cx.theme().danger
                                    } else {
                                        cx.theme().muted_foreground
                                    })
                                    .child(if index == 0 {
                                        format!("{bytes} / {} bytes", memory_files::L1_MAX_BYTES)
                                    } else {
                                        format!("{bytes} bytes")
                                    }),
                            ),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(hint),
                    )
                    .child(
                        Textarea::new(&self.editors[index])
                            .aria_label(title)
                            .h(px(220.))
                            .disabled(self.busy),
                    )
                    .child(
                        div().flex().justify_end().child(
                            Button::new(SharedString::from(format!("memory-save-{index}")))
                                .primary()
                                .label("保存")
                                .disabled(
                                    self.busy
                                        || over_limit
                                        || text.as_ref() == self.confirmed[index],
                                )
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.load(Some(index), window, cx)
                                })),
                        ),
                    ),
            );
        }
        crate::kivio_settings::page_frame(
            "记忆",
            "L1 注入，L2 按需读取。",
            body.into_any_element(),
            cx,
        )
    }
}
