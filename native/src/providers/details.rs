//! Model-level overrides stay in the existing provider draft and atomic writer.
use super::*;
use gpui_kit::component::input::{Textarea, TextareaState};
use std::collections::BTreeMap;
const FIELDS: &[(&str, &str)] = &[
    ("displayName", "显示名称"),
    ("contextWindow", "上下文长度"),
    ("maxOutput", "最大输出"),
    ("temperature", "采样温度（0–2）"),
    ("input", "输入价格"),
    ("output", "输出价格"),
    ("cachedInput", "缓存输入价格"),
];
const CAPABILITIES: &[(&str, &str)] = &[
    ("vision", "图片理解"),
    ("videoInput", "视频理解"),
    ("functionCalling", "工具调用"),
    ("reasoning", "思考"),
    ("streaming", "流式输出"),
    ("webSearch", "内置搜索"),
    ("imageGeneration", "图片生成"),
    ("videoGeneration", "视频生成"),
];
const EFFORTS: &[&str] = &["low", "medium", "high", "xhigh", "max"];
pub(super) struct ModelDetails {
    model: String,
    value: Value,
    fields: BTreeMap<&'static str, Entity<InputState>>,
    extra: Entity<TextareaState>,
    temperature_edited: bool,
    pricing_edited: bool,
    capabilities: Value,
    _inputs: Vec<Subscription>,
}
impl ProviderSettings {
    pub(super) fn open_model(
        &mut self,
        model: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy {
            return;
        }
        if !self.capture_pending_editor(window, cx) {
            return;
        }
        let Some(provider) = self
            .draft
            .iter()
            .find(|p| Some(&p.id) == self.editing.as_ref())
        else {
            return;
        };
        let value = serde_json::to_value(
            provider
                .model_overrides
                .get(&model)
                .cloned()
                .unwrap_or_default(),
        )
        .unwrap();
        let defaults = serde_json::to_value(
            kivio_model_runtime::model_metadata::database_model_info(provider, &model),
        )
        .unwrap();
        let capabilities = serde_json::to_value(
            kivio_model_runtime::model_metadata::capabilities_for_model(provider, &model),
        )
        .unwrap();
        let mut fields = BTreeMap::new();
        let mut inputs = vec![];
        for &(key, _) in FIELDS {
            let current = if ["input", "output", "cachedInput"].contains(&key) {
                if value["pricing"].is_null() {
                    &defaults["pricing"][key]
                } else {
                    &value["pricing"][key]
                }
            } else {
                &value[key]
            };
            let text = if current.is_null() {
                String::new()
            } else if let Some(text) = current.as_str() {
                text.into()
            } else {
                current.to_string()
            };
            let inherited = if key == "temperature" && value["omitTemperature"] == true {
                "不发送 temperature".into()
            } else if let Some(value) = defaults.get(key).filter(|value| !value.is_null()) {
                crate::ui_format!("自动 · {}", "Auto · {}",
                    value
                        .as_str()
                        .map(str::to_owned)
                        .unwrap_or_else(|| value.to_string())
                )
            } else {
                "跟随模型库".into()
            };
            let input = cx.new(|cx| {
                InputState::new(window, cx)
                    .default_value(text)
                    .placeholder(inherited)
            });
            inputs.push(cx.subscribe(&input, move |this, _, event, cx| {
                if matches!(event, InputEvent::Change) {
                    if ["input", "output", "cachedInput"].contains(&key) {
                        if let Some(details) = &mut this.model_details {
                            details.pricing_edited = true;
                        }
                    }
                    if key == "temperature" {
                        if let Some(details) = &mut this.model_details {
                            details.temperature_edited = true;
                        }
                    }
                    this.dirty = true;
                    this.error = None;
                    cx.notify();
                }
            }));
            fields.insert(key, input);
        }
        let extra_text = value
            .get("extraBody")
            .filter(|v| !v.is_null())
            .map(|v| serde_json::to_string_pretty(v).unwrap())
            .unwrap_or_default();
        let extra = cx.new(|cx| {
            TextareaState::new(window, cx)
                .rows(6)
                .default_value(extra_text)
        });
        inputs.push(cx.subscribe(&extra, |this, _, event, cx| {
            if matches!(event, InputEvent::Change) {
                this.dirty = true;
                this.error = None;
                cx.notify();
            }
        }));
        self.model_details = Some(ModelDetails {
            model,
            value,
            fields,
            extra,
            temperature_edited: false,
            pricing_edited: false,
            capabilities,
            _inputs: inputs,
        });
        self.error = None;
        cx.notify();
    }
    pub(super) fn capture_model(&mut self, cx: &Context<Self>) -> Result<(), String> {
        let Some(details) = &self.model_details else {
            return Ok(());
        };
        let mut value = details.value.clone();
        if details.pricing_edited && !value["pricing"].is_object() {
            value["pricing"] = json!({});
        }
        for &(key, label) in FIELDS {
            let text = details.fields[key].read(cx).value();
            let text = text.trim();
            let parsed = if text.is_empty() {
                Value::Null
            } else if key == "displayName" {
                json!(text)
            } else if ["contextWindow", "maxOutput"].contains(&key) {
                let number = text
                    .parse::<u64>()
                    .ok()
                    .filter(|n| *n > 0)
                    .ok_or_else(|| crate::ui_format!("{label} 必须是正整数", "{label} must be a positive integer"))?;
                json!(number)
            } else {
                let number = text
                    .parse::<f64>()
                    .ok()
                    .filter(|v| v.is_finite() && *v >= 0. && (key != "temperature" || *v <= 2.))
                    .ok_or_else(|| crate::ui_format!("{label} 无效", "Invalid {label}"))?;
                json!(number)
            };
            if ["input", "output", "cachedInput"].contains(&key) {
                if details.pricing_edited {
                    value["pricing"][key] = parsed;
                }
            } else {
                value[key] = parsed;
            }
        }
        if value["pricing"]
            .as_object()
            .is_some_and(|values| values.values().all(Value::is_null))
        {
            value["pricing"] = Value::Null;
        }
        if details.temperature_edited {
            value["omitTemperature"] = json!(value["temperature"].is_null());
        }
        let extra = details.extra.read(cx).value();
        value["extraBody"] = if extra.trim().is_empty() {
            Value::Null
        } else {
            let parsed: Value = serde_json::from_str(extra.trim())
                .map_err(|e| crate::ui_format!("额外请求体 JSON 无效：{e}", "Invalid extra request body JSON: {e}"))?;
            if !parsed.is_object() {
                return Err("额外请求体必须是 JSON 对象".into());
            }
            parsed
        };
        let info = serde_json::from_value(value).map_err(|e| crate::ui_format!("模型配置无效：{e}", "Invalid model configuration: {e}"))?;
        if let Some(provider) = self
            .draft
            .iter_mut()
            .find(|p| Some(&p.id) == self.editing.as_ref())
        {
            provider.model_overrides.insert(details.model.clone(), info);
        }
        Ok(())
    }
    pub(super) fn model_detail_page(&self, cx: &Context<Self>) -> AnyElement {
        let details = self.model_details.as_ref().unwrap();
        let mut body = div().v_flex().gap_4().child(div().flex().child(Button::new("model-details-back").small().ghost().label(crate::i18n::text("返回供应商")).on_click(cx.listener(|this, _, _, cx| {
            if let Err(error) = this.capture_model(cx) { this.error = Some(error); } else { this.model_details = None; }
            cx.notify();
        })))).child(div().text_base().child(details.model.clone())).child(div().text_sm().text_color(cx.theme().muted_foreground).child(crate::i18n::text("留空使用模型库默认值。修改温度后清空会禁止发送 temperature；能力可单独跟随模型库。")));
        for &(key, label) in FIELDS {
            if key == "input" {
                body = body.child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(crate::i18n::text("价格 · USD / 百万 tokens")),
                );
            }
            body = body.child(field(
                label,
                Input::new(&details.fields[key])
                    .small()
                    .aria_label(crate::i18n::text(label))
                    .disabled(self.busy),
            ));
        }
        body = body.child(div().text_base().child(crate::i18n::text("模型能力")));
        for &(key, label) in CAPABILITIES {
            let current = details.value["capabilities"][key].as_bool();
            let owner = cx.weak_entity();
            body = body.child(field(
                label,
                Button::new(SharedString::from(format!("capability-{key}")))
                    .small()
                    .label(match current {
                        None => match details.capabilities[key].as_bool() {
                            Some(true) => "跟随模型库（支持）",
                            Some(false) => "跟随模型库（不支持）",
                            None => "跟随模型库（未知）",
                        },
                        Some(true) => "支持",
                        Some(false) => "不支持",
                    })
                    .disabled(self.busy)
                    .dropdown_caret(true)
                    .dropdown_menu(move |mut menu, _, _| {
                        for (value, label) in [
                            (None, "跟随模型库"),
                            (Some(true), "支持"),
                            (Some(false), "不支持"),
                        ] {
                            let owner = owner.clone();
                            menu = menu.item(
                                PopupMenuItem::new(label)
                                    .checked(value == current)
                                    .on_click(move |_, _, cx| {
                                        let _ = owner.update(cx, |this, cx| {
                                            if let Some(details) = &mut this.model_details {
                                                if !details.value["capabilities"].is_object() {
                                                    details.value["capabilities"] = json!({});
                                                }
                                                details.value["capabilities"][key] = json!(value);
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
        let follow = details.value["reasoningEfforts"].is_null();
        body = body.child(
            Checkbox::new("effort-follow")
                .label(crate::i18n::text("思考等级跟随模型库"))
                .checked(follow)
                .disabled(self.busy)
                .on_click(cx.listener(|this, checked, _, cx| {
                    let default = this
                        .draft
                        .iter()
                        .find(|p| Some(&p.id) == this.editing.as_ref())
                        .zip(this.model_details.as_ref())
                        .map(|(p, d)| {
                            kivio_model_runtime::model_metadata::reasoning_efforts_for_model(
                                Some(p),
                                &d.model,
                            )
                        })
                        .unwrap_or_default();
                    if let Some(details) = &mut this.model_details {
                        details.value["reasoningEfforts"] = if *checked {
                            Value::Null
                        } else {
                            json!(default)
                        };
                        this.dirty = true;
                        cx.notify();
                    }
                })),
        );
        if !follow {
            let mut efforts = div().flex().flex_wrap().gap_2();
            for &effort in EFFORTS {
                let checked = details.value["reasoningEfforts"]
                    .as_array()
                    .is_some_and(|values| values.iter().any(|v| v.as_str() == Some(effort)));
                efforts = efforts.child(
                    Button::new(SharedString::from(format!("model-effort-{effort}")))
                        .small()
                        .label(effort)
                        .selected(checked)
                        .disabled(self.busy)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(details) = &mut this.model_details {
                                let mut selected: Vec<String> = serde_json::from_value(
                                    details.value["reasoningEfforts"].clone(),
                                )
                                .unwrap_or_default();
                                if checked {
                                    selected.retain(|v| v != effort);
                                } else {
                                    selected.push(effort.into());
                                }
                                details.value["reasoningEfforts"] = json!(selected);
                                this.dirty = true;
                                cx.notify();
                            }
                        })),
                );
            }
            body = body.child(efforts).child(
                div()
                    .text_xs()
                    .child(crate::i18n::text("全部取消表示此模型不发送思考等级字段。")),
            );
        }
        body.child(div().text_base().child(crate::i18n::text("额外请求体（JSON 对象）")))
            .child(
                Textarea::new(&details.extra)
                    .aria_label("额外请求体 JSON")
                    .disabled(self.busy),
            )
            .child(
                div().flex().child(
                    Button::new("reset-model-overrides")
                        .small()
                        .label(crate::i18n::text("清除本模型自定义配置"))
                        .disabled(self.busy)
                        .on_click(cx.listener(|this, _, _, cx| {
                            if let Some(details) = this.model_details.take() {
                                if let Some(provider) = this
                                    .draft
                                    .iter_mut()
                                    .find(|p| Some(&p.id) == this.editing.as_ref())
                                {
                                    provider.model_overrides.remove(&details.model);
                                }
                                this.dirty = true;
                                cx.notify();
                            }
                        })),
                ),
            )
            .into_any_element()
    }
}
