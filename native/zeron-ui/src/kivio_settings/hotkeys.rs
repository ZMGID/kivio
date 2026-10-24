use super::*;
use gpui_kit::component::Sizable;
use serde_json::json;

const KEYS: &[(&str, &str)] = &[
    ("输入翻译", "/hotkey"),
    ("打开聊天", "/chatHotkey"),
    ("关闭聊天", "/closeChatHotkey"),
    ("截图翻译", "/screenshotTranslation/hotkey"),
    ("选中文本翻译", "/screenshotTranslation/textHotkey"),
    ("替换翻译", "/screenshotTranslation/replaceHotkey"),
    ("截图标注", "/screenshotAnnotate/hotkey"),
    ("Lens", "/lens/hotkey"),
];
impl SettingsPage {
    pub(super) fn shortcuts(&self, theme: &Theme, cx: &Context<Self>) -> Div {
        let rows = KEYS
            .iter()
            .map(|&(label, path)| {
                let recording = self.recording == Some(path);
                let value = self
                    .field_value(path)
                    .as_str()
                    .unwrap_or_default()
                    .to_string();
                row(
                    label,
                    div()
                        .flex()
                        .gap_2()
                        .child(
                            Button::new(path)
                                .small()
                                .label(if recording {
                                    crate::i18n::text("请按组合键；Esc 取消").into()
                                } else if value.is_empty() {
                                    crate::i18n::text("录制快捷键…").into()
                                } else {
                                    value
                                })
                                .disabled(self.busy)
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.record_hotkey(path, window, cx)
                                })),
                        )
                        .child(
                            Button::new(SharedString::from(format!("clear-{path}")))
                                .small()
                                .label(crate::i18n::text("清除"))
                                .disabled(self.busy)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.set_fields(vec![(path, json!(""))], cx)
                                })),
                        )
                        .into_any_element(),
                    theme,
                )
            })
            .collect();
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(card("全局快捷键", rows, theme))
            .when_some(crate::global_hotkeys::status(cx), |view, error| view.child(div().text_sm().text_color(theme.danger).child(error)))
            .child(
                div().flex().child(
                    Button::new("reset-hotkeys")
                        .small()
                        .label(crate::i18n::text("恢复默认"))
                        .disabled(self.busy)
                        .on_click(cx.listener(|this, _, _, cx| {
                            let defaults: Settings =
                                serde_json::from_value(json!({})).expect("Settings defaults");
                            let defaults =
                                serde_json::to_value(defaults).expect("serialize settings");
                            this.set_fields(
                                KEYS.iter()
                                    .map(|&(_, path)| {
                                        (path, defaults.pointer(path).cloned().unwrap_or(json!("")))
                                    })
                                    .collect(),
                                cx,
                            );
                        })),
                ),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(theme.text_muted)
                    .child(crate::i18n::text("输入翻译、选中文本翻译、截图翻译、Lens、打开聊天与关闭聊天已接入系统全局快捷键。替换翻译与截图标注的入口仍在接入。")),
            )
    }
    fn record_hotkey(&mut self, path: &'static str, window: &mut Window, cx: &mut Context<Self>) {
        self.recording = Some(path);
        self.error = None;
        let owner = cx.weak_entity();
        self.recording_listener = Some(cx.intercept_keystrokes(move |event, window, cx| {
            let _ = owner.update(cx, |this, cx| {
                if !this.focus.is_focused(window) {
                    return;
                }
                let key = &event.keystroke;
                if key.key.eq_ignore_ascii_case("escape") {
                    this.recording = None;
                    this.recording_listener = None;
                } else if !matches!(key.key.as_str(), "shift" | "control" | "alt" | "cmd" | "fn") {
                    let m = key.modifiers;
                    if !(m.control || m.alt || m.platform) {
                        this.error =
                            Some("请同时按下 Command、Control 或 Option 等修饰键。".into());
                    } else {
                        let mut parts = Vec::new();
                        if m.control {
                            parts.push("Control".to_string());
                        }
                        if m.alt {
                            parts.push("Alt".to_string());
                        }
                        let (key_name, shifted) = shortcut_key(&key.key);
                        if m.shift || shifted {
                            parts.push("Shift".to_string());
                        }
                        if m.platform {
                            parts.push("CommandOrControl".to_string());
                        }
                        parts.push(key_name);
                        let combo = parts.join("+");
                        let conflict = KEYS.iter().find(|&&(_, candidate)| {
                            candidate != path
                                && this
                                    .field_value(candidate)
                                    .as_str()
                                    .is_some_and(|value| hotkeys_equal(value, &combo))
                        });
                        if let Some((label, _)) = conflict {
                            this.error = Some(crate::ui_format!("此组合键已用于“{label}”，请换一个组合键。", "This shortcut is already used by “{label}”. Choose another combination."));
                        } else {
                            this.recording = None;
                            this.recording_listener = None;
                            this.set_fields(vec![(path, json!(combo))], cx);
                        }
                    }
                }
                cx.stop_propagation();
                cx.notify();
            });
        }));
        self.recording_blur = Some(cx.on_blur(&self.focus, window, |this, _, cx| {
            this.recording = None;
            this.recording_listener = None;
            cx.notify();
        }));
        window.focus(&self.focus, cx);
        cx.notify();
    }
}

// GPUI represents shifted punctuation as the resulting glyph, including when
// its normalized modifiers no longer contain Shift. Store accelerator names.
fn shortcut_key(key: &str) -> (String, bool) {
    let shifted = match key {
        "!" => Some("1"),
        "@" => Some("2"),
        "#" => Some("3"),
        "$" => Some("4"),
        "%" => Some("5"),
        "^" => Some("6"),
        "&" => Some("7"),
        "*" => Some("8"),
        "(" => Some("9"),
        ")" => Some("0"),
        "_" => Some("Minus"),
        "+" => Some("Equal"),
        "{" => Some("BracketLeft"),
        "}" => Some("BracketRight"),
        "|" => Some("Backslash"),
        ":" => Some("Semicolon"),
        "\"" => Some("Quote"),
        "<" => Some("Comma"),
        ">" => Some("Period"),
        "?" => Some("Slash"),
        "~" => Some("Backquote"),
        _ => None,
    };
    if let Some(base) = shifted {
        return (base.into(), true);
    }
    (
        match key {
            "space" => "Space".into(),
            "enter" => "Enter".into(),
            "backspace" => "Backspace".into(),
            "delete" => "Delete".into(),
            "tab" => "Tab".into(),
            "up" => "Up".into(),
            "down" => "Down".into(),
            "left" => "Left".into(),
            "right" => "Right".into(),
            "pageup" => "PageUp".into(),
            "pagedown" => "PageDown".into(),
            "home" => "Home".into(),
            "end" => "End".into(),
            "-" => "Minus".into(),
            "=" => "Equal".into(),
            "[" => "BracketLeft".into(),
            "]" => "BracketRight".into(),
            "\\" => "Backslash".into(),
            ";" => "Semicolon".into(),
            "'" => "Quote".into(),
            "," => "Comma".into(),
            "." => "Period".into(),
            "/" => "Slash".into(),
            "`" => "Backquote".into(),
            other => other.to_uppercase(),
        },
        false,
    )
}

// Compare physical chords, including imported CommandOrControl aliases and
// modifier ordering, without changing the portable value stored on disk.
fn hotkeys_equal(left: &str, right: &str) -> bool {
    let normalize = |value: &str| {
        let mut parts: Vec<String> = value
            .split('+')
            .filter(|part| !part.trim().is_empty())
            .map(|part| match part.trim().to_ascii_lowercase().as_str() {
                "commandorcontrol" | "cmd" | "command" => {
                    if cfg!(target_os = "macos") {
                        "super".into()
                    } else {
                        "control".into()
                    }
                }
                "ctrl" => "control".into(),
                "meta" => "super".into(),
                "opt" | "option" => "alt".into(),
                other => other.into(),
            })
            .collect();
        parts.sort();
        parts
    };
    normalize(left) == normalize(right)
}
