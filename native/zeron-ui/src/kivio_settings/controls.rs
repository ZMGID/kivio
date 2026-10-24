//! Retained field editors for the Kivio settings document. Each commit uses the
//! confirmed value as its baseline; unrelated fields never overwrite one another.
use super::*;
use gpui_kit::component::Sizable;
use kivio_model_runtime::native_preferences::FieldEdit;
use serde_json::{Value, json};

impl SettingsPage {
    pub(super) fn field_feedback(&self, field: &str, theme: &Theme) -> Div {
        let Some((key, result)) = &self.feedback else {
            return div();
        };
        if key != field {
            return div();
        }
        let message = match result {
            Ok(()) => "已保存到本机。".to_string(),
            Err(error) => error.clone(),
        };
        div()
            .text_sm()
            .text_color(if result.is_err() {
                theme.danger
            } else {
                theme.text_muted
            })
            .child(message)
    }
    pub(super) fn field_value(&self, path: &str) -> Value {
        self.settings_value
            .pointer(path)
            .cloned()
            .unwrap_or(Value::Null)
    }
    pub(super) fn set_fields(&mut self, values: Vec<(&str, Value)>, cx: &mut Context<Self>) {
        let edits = values
            .into_iter()
            .map(|(path, after)| FieldEdit {
                path: path.into(),
                // Retained text can predate an unrelated save's returned Settings.
                // Preserve that editor's original confirmed value until it saves.
                before: self.field_baselines.get(path).cloned().unwrap_or_else(|| self.field_value(path)),
                after,
            })
            .collect();
        self.save(Change::Fields(edits), cx);
    }
    pub(super) fn toggle(
        &self,
        label: &'static str,
        path: &'static str,
        invert: bool,
        theme: &Theme,
        cx: &Context<Self>,
    ) -> Div {
        let checked = self.field_value(path).as_bool().unwrap_or(false) ^ invert;
        row(
            label,
            Switch::new(path)
                .accessibility_label(crate::i18n::text(label))
                .checked(checked)
                .disabled(self.busy)
                .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                    this.set_fields(vec![(path, json!(*checked ^ invert))], cx);
                }))
                .into_any_element(),
            theme,
        )
    }
    pub(super) fn choice(
        &self,
        label: &'static str,
        path: &'static str,
        choices: &'static [(&'static str, &'static str)],
        theme: &Theme,
        cx: &Context<Self>,
    ) -> Div {
        let current = self
            .field_value(path)
            .as_str()
            .unwrap_or_default()
            .to_owned();
        let title = choices
            .iter()
            .find(|(value, _)| *value == current)
            .map(|(_, title)| title.to_string())
            .unwrap_or_else(|| {
                if current.is_empty() {
                    "未设置".into()
                } else {
                    current.clone()
                }
            });
        let owner = cx.weak_entity();
        row(
            label,
            Button::new(path)
                .small()
                .label(crate::i18n::display(title))
                .dropdown_caret(true)
                .disabled(self.busy)
                .dropdown_menu(move |mut menu, _, _| {
                    for &(value, title) in choices {
                        let owner = owner.clone();
                        menu = menu.item(
                            PopupMenuItem::new(crate::i18n::text(title))
                                .checked(value == current)
                                .on_click(move |_, _, cx| {
                                    let _ = owner.update(cx, |this, cx| {
                                        this.set_fields(vec![(path, json!(value))], cx)
                                    });
                                }),
                        );
                    }
                    menu
                })
                .into_any_element(),
            theme,
        )
    }
    pub(super) fn text_field(
        &self,
        label: &'static str,
        path: &'static str,
        hint: &'static str,
        theme: &Theme,
        cx: &Context<Self>,
    ) -> Div {
        let Some(input) = self.fields.get(path) else {
            return div();
        };
        div()
            .flex()
            .flex_col()
            .gap_2()
            .py_3()
            .child(crate::i18n::display(label))
            .when(!hint.is_empty(), |view| {
                view.child(div().text_xs().text_color(theme.text_muted).child(hint))
            })
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .when(self.field_value(path).is_number(), |view| {
                                view.max_w(px(120.))
                            })
                            .min_w_0()
                            .child(
                                Input::new(input)
                                    .small()
                                    .aria_label(crate::i18n::text(label))
                                    .disabled(self.busy),
                            ),
                    )
                    .when(matches!(path, "/imageArchivePath" | "/chatTools/nativeTools/workingDirectory"), |view| {
                        view.child(Button::new(SharedString::from(format!("choose{path}")))
                            .small().label(crate::i18n::text("选择目录…")).disabled(self.busy)
                            .on_click(cx.listener(move |this, _, window, cx| this.choose_directory(path, window, cx))))
                            .child(Button::new(SharedString::from(format!("reset{path}")))
                                .small().label(crate::i18n::text("恢复默认")).disabled(self.busy)
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    let value = if path == "/imageArchivePath" { String::new() }
                                        else { kivio_model_runtime::tool_settings::default_chat_working_directory() };
                                    this.apply_directory(path, value, window, cx);
                                })))
                    })
                    .child(
                        Button::new(SharedString::from(format!("save{path}")))
                            .small()
                            .label(crate::i18n::text("保存"))
                            .disabled(self.busy)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let Some(input) = this.fields.get(path) else {
                                    return;
                                };
                                let text = input.read(cx).value().to_string();
                                let value = if this.field_value(path).is_number() {
                                    match text.parse::<u64>() {
                                        Ok(n) => json!(n),
                                        Err(_) => {
                                            this.error = Some("请输入有效的非负整数。".into());
                                            this.feedback = Some((
                                                path.into(),
                                                Err("请输入有效的非负整数。".into()),
                                            ));
                                            cx.notify();
                                            return;
                                        }
                                    }
                                } else {
                                    json!(text)
                                };
                                this.set_fields(vec![(path, value)], cx);
                            })),
                    ),
            )
            .child(self.field_feedback(path, theme))
    }
    fn apply_directory(&mut self, path: &'static str, value: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy { return; }
        if let Some(input) = self.fields.get(path) {
            input.update(cx, |input, cx| input.set_value(value.clone(), window, cx));
            self.set_fields(vec![(path, json!(value))], cx);
        }
    }
    fn choose_directory(&mut self, path: &'static str, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy { return; }
        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: false, directories: true, multiple: false, prompt: Some("选择目录".into()),
        });
        let handle = window.window_handle();
        self.busy = true;
        self.directory_prompt = Some(cx.spawn(async move |owner, cx| {
            let selected = prompt.await;
            let _ = cx.update_window(handle, |_, window, cx| {
                let _ = owner.update(cx, |this, cx| {
                    this.busy = false;
                    match selected {
                        Ok(Ok(Some(paths))) => if let Some(selected) = paths.into_iter().next() {
                            this.apply_directory(path, selected.to_string_lossy().into_owned(), window, cx);
                        },
                        Ok(Ok(None)) => {},
                        Ok(Err(error)) => this.feedback = Some((path.into(), Err(error.to_string()))),
                        Err(error) => this.feedback = Some((path.into(), Err(error.to_string()))),
                    }
                    cx.notify();
                });
            });
        }));
        cx.notify();
    }
    pub(super) fn model_pair(
        &self,
        label: &'static str,
        provider_path: &'static str,
        model_path: &'static str,
        inherit: bool,
        theme: &Theme,
        cx: &Context<Self>,
    ) -> Div {
        let provider = self
            .field_value(provider_path)
            .as_str()
            .unwrap_or_default()
            .to_owned();
        let model = self
            .field_value(model_path)
            .as_str()
            .unwrap_or_default()
            .to_owned();
        let choices = self.providers.read(cx).models();
        let title = choices
            .iter()
            .find(|choice| choice.provider_id == provider && choice.model == model)
            .map(|choice| choice.label.to_string())
            .unwrap_or_else(|| {
                if provider.is_empty() {
                    if inherit {
                        "跟随输入翻译".into()
                    } else {
                        "选择模型".into()
                    }
                } else {
                    crate::ui_format!("{provider} · {model}（不可用）", "{provider} · {model} (unavailable)")
                }
            });
        let owner = cx.weak_entity();
        row(
            label,
            Button::new(model_path)
                .small()
                .label(crate::i18n::display(title))
                .dropdown_caret(true)
                .disabled(self.busy || choices.is_empty() && !inherit)
                .dropdown_menu(move |mut menu, _, _| {
                    if inherit {
                        let owner = owner.clone();
                        menu = menu.item(
                            PopupMenuItem::new(crate::i18n::text("跟随输入翻译"))
                                .checked(provider.is_empty())
                                .on_click(move |_, _, cx| {
                                    let _ = owner.update(cx, |this, cx| {
                                        this.set_fields(
                                            vec![
                                                (provider_path, json!("")),
                                                (model_path, json!("")),
                                            ],
                                            cx,
                                        )
                                    });
                                }),
                        );
                    }
                    for choice in &choices {
                        let owner = owner.clone();
                        let choice = choice.clone();
                        menu = menu.item(
                            PopupMenuItem::new(choice.label.clone())
                                .checked(choice.provider_id == provider && choice.model == model)
                                .on_click(move |_, _, cx| {
                                    let _ = owner.update(cx, |this, cx| {
                                        this.set_fields(
                                            vec![
                                                (provider_path, json!(choice.provider_id)),
                                                (model_path, json!(choice.model)),
                                            ],
                                            cx,
                                        )
                                    });
                                }),
                        );
                    }
                    menu
                })
                .into_any_element(),
            theme,
        )
    }
}

pub(super) const TEXT_FIELDS: &[&str] = &[
    "/screenshotTranslation/cardWidth",
    "/imageArchivePath",
    "/chatTools/nativeTools/workingDirectory",
    "/chat/userAvatar",
    "/chatTools/subAgentConcurrency",
    "/chatTools/playwrightExtensionToken",
];
