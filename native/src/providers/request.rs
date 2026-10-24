//! Request configuration uses the same retained provider draft and writer.
use super::*;
impl ProviderSettings {
    pub(super) fn watch_headers(&mut self, cx: &mut Context<Self>) {
        self.header_subscriptions = self
            .headers
            .iter()
            .flat_map(|(key, value)| [key, value])
            .map(|input| {
                cx.subscribe(input, |this, _, event, cx| {
                    if matches!(event, InputEvent::Change) {
                        this.dirty = true;
                        this.error = None;
                        cx.notify();
                    }
                })
            })
            .collect();
    }

    pub(super) fn request_page(&self, provider: &ModelProvider, cx: &Context<Self>) -> AnyElement {
        let disabled = self.busy;
        let mut body = div()
            .v_flex()
            .gap_4()
            .child(
                div().flex().child(
                    Button::new("request-back")
                        .small()
                        .ghost()
                        .label(crate::i18n::text("返回供应商"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.request_page = false;
                            cx.notify();
                        })),
                ),
            )
            .child(
                div()
                    .text_size(px(14.))
                    .font_weight(FontWeight::MEDIUM)
                    .child(crate::ui_format!("{} · 请求配置", "{} · Request configuration", provider.name)),
            );
        for (id, label, checked) in [
            ("proxy", "跟随系统代理", provider.request.use_system_proxy),
            ("gzip", "压缩请求体（gzip）", provider.compress_request_body),
        ] {
            body = body.child(
                gpui_kit::component::switch::Switch::new(id)
                    .label(crate::i18n::display(label))
                    .checked(checked)
                    .disabled(disabled)
                    .on_click(cx.listener(move |this, value, _, cx| {
                        if let Some(provider) = this
                            .draft
                            .iter_mut()
                            .find(|p| Some(&p.id) == this.editing.as_ref())
                        {
                            if id == "proxy" {
                                provider.request.use_system_proxy = *value;
                            } else {
                                provider.compress_request_body = *value;
                            }
                            this.dirty = true;
                            cx.notify();
                        }
                    })),
            );
        }
        for (id, label, current, options) in [
            (
                "cache",
                "Prompt 缓存",
                provider.request.prompt_cache_retention.clone(),
                vec![
                    ("none", "关闭"),
                    ("short", "短期缓存"),
                    ("long", "长期缓存"),
                ],
            ),
            (
                "identity",
                "CLI 请求身份",
                provider.request.cli_identity.clone(),
                vec![
                    ("", "关闭"),
                    ("claude_code", "Claude Code"),
                    ("codex", "Codex"),
                    ("grok", "Grok"),
                ],
            ),
        ] {
            let title = options
                .iter()
                .find(|(key, _)| *key == current)
                .map(|(_, label)| *label)
                .unwrap_or("关闭");
            let owner = cx.weak_entity();
            body = body.child(field(
                label,
                Button::new(id)
                    .small()
                    .label(title)
                    .dropdown_caret(true)
                    .disabled(disabled)
                    .dropdown_menu(move |mut menu, _, _| {
                        for &(value, label) in &options {
                            let owner = owner.clone();
                            menu = menu.item(
                                PopupMenuItem::new(label)
                                    .checked(value == current)
                                    .on_click(move |_, _, cx| {
                                        let _ = owner.update(cx, |this, cx| {
                                            if let Some(provider) = this
                                                .draft
                                                .iter_mut()
                                                .find(|p| Some(&p.id) == this.editing.as_ref())
                                            {
                                                if id == "cache" {
                                                    provider.request.prompt_cache_retention =
                                                        value.into();
                                                } else {
                                                    provider.request.cli_identity = value.into();
                                                }
                                                this.dirty = true;
                                                cx.notify();
                                            }
                                        });
                                    }),
                            );
                        }
                        menu
                    }),
            ));
        }
        body = body
            .child(field(
                "身份版本号（留空使用默认值）",
                Input::new(&self.identity_version)
                    .small()
                    .aria_label("CLI 身份版本号")
                    .disabled(disabled),
            ))
            .child(div().text_base().child(crate::i18n::text("自定义请求头")))
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(crate::i18n::text("附加到此供应商的请求。认证、Content-Type 等保留请求头不能覆盖。")),
            );
        for (ix, (key, value)) in self.headers.iter().enumerate() {
            body = body.child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        div().flex_1().min_w_0().child(
                            Input::new(key)
                                .small()
                                .id(("request-header-key", ix))
                                .aria_label(crate::ui_format!("请求头 {} 名称", "Header {} name", ix + 1))
                                .disabled(disabled),
                        ),
                    )
                    .child(
                        div().flex_1().min_w_0().child(
                            Input::new(value)
                                .small()
                                .id(("request-header-value", ix))
                                .aria_label(crate::ui_format!("请求头 {} 值", "Header {} value", ix + 1))
                                .disabled(disabled),
                        ),
                    )
                    .child(
                        Button::new(("remove-header", ix))
                            .small()
                            .label(crate::i18n::text("移除"))
                            .disabled(disabled)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.headers.remove(ix);
                                this.watch_headers(cx);
                                this.dirty = true;
                                cx.notify();
                            })),
                    ),
            );
        }
        body.child(
            div().flex().child(
                Button::new("add-header")
                    .small()
                    .label(crate::i18n::text("添加请求头"))
                    .disabled(disabled)
                    .on_click(cx.listener(|this, _, window, cx| {
                        let key = cx.new(|cx| InputState::new(window, cx).placeholder("X-Header"));
                        let value = cx.new(|cx| InputState::new(window, cx).placeholder("Value"));
                        this.headers.push((key, value));
                        this.watch_headers(cx);
                        this.dirty = true;
                        cx.notify();
                    })),
            ),
        )
        .into_any_element()
    }
}
