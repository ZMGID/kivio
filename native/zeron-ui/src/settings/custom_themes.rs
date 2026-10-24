//! Kivio semantic palettes rendered through the existing Zeron theme domain.
use gpui_kit::component::{
    ActiveTheme, Disableable, Selectable, Sizable, StyledExt, WindowExt,
    button::Button,
    input::{Input, InputEvent, InputState},
};
use gpui_kit::{prelude::*, *};
use kivio_model_runtime::{
    native_preferences::{self, Change},
    settings::{Settings, ThemeDefinition},
};
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::PathBuf};
const COLORS: &[(&str, &str)] = &[
    ("surface", "主背景"),
    ("surfaceSoft", "侧栏背景"),
    ("surfaceMuted", "次级背景"),
    ("surfaceHover", "悬停背景"),
    ("surfaceActive", "选中背景"),
    ("surfaceTitlebar", "标题栏背景"),
    ("text", "主文字"),
    ("textMuted", "次级文字"),
    ("textFaint", "弱化文字"),
    ("border", "边框"),
    ("borderStrong", "强调边框"),
    ("accent", "强调色"),
    ("accentHover", "强调悬停"),
    ("accentSoft", "强调背景"),
    ("onAccent", "强调色上文字"),
    ("danger", "危险状态"),
    ("dangerSoft", "危险背景"),
];
pub fn install(settings: &Settings) {
    let builtins = zeron_theme::ThemeRegistry::builtin();
    let mut families: Vec<_> = zeron_theme::ThemeRegistry::active()
        .families
        .into_iter()
        .filter(|family| {
            !builtins.families.iter().any(|entry| entry.id == family.id)
                && !family.id.starts_with("kivio-")
        })
        .collect();
    families.extend(
        settings
            .custom_themes
            .iter()
            .filter_map(|theme| zeron_theme::kivio_family(&serde_json::to_value(theme).ok()?).ok()),
    );
    zeron_theme::replace_custom_families(families);
}
pub struct CustomThemes {
    english_placeholders: bool,
    root: PathBuf,
    themes: Vec<ThemeDefinition>,
    selected: Option<ThemeDefinition>,
    draft: Option<Value>,
    baseline: Option<ThemeDefinition>,
    mode: &'static str,
    name: Entity<InputState>,
    colors: BTreeMap<&'static str, Entity<InputState>>,
    busy: bool,
    error: Option<String>,
    task: Option<Task<()>>,
    _inputs: Vec<Subscription>,
}
impl CustomThemes {
    pub fn new(root: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let name = cx.new(|cx| InputState::new(window, cx).placeholder(crate::i18n::text("主题名称")));
        let colors = COLORS
            .iter()
            .map(|(key, _)| {
                (
                    *key,
                    cx.new(|cx| InputState::new(window, cx).placeholder("#rrggbb")),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let mut subscriptions = vec![];
        for input in colors.values() {
            subscriptions.push(cx.subscribe(input, |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }));
        }
        let mut this = Self {
            english_placeholders: crate::i18n::is_english(),
            root,
            themes: vec![],
            selected: None,
            draft: None,
            baseline: None,
            mode: "light",
            name,
            colors,
            busy: false,
            error: None,
            task: None,
            _inputs: subscriptions,
        };
        this.reload(None, cx);
        this
    }
    pub(crate) fn is_busy(&self, _: &App) -> bool {
        self.busy
    }
    pub(crate) fn has_unsaved_changes(&self, cx: &App) -> bool {
        let Some(draft) = self.capture(cx) else {
            return false;
        };
        self.baseline
            .as_ref()
            .and_then(|theme| serde_json::to_value(theme).ok())
            .as_ref()
            != Some(&draft)
    }
    pub(crate) fn editing(&self) -> bool {
        self.busy || self.draft.is_some()
    }
    pub(crate) fn apply_builtin(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.editing() {
            return;
        }
        let builtins = builtin_themes();
        if let Some(theme) = builtins.iter().find(|theme| theme.id == id).cloned() {
            self.selected = None;
            self.apply(theme, cx);
        }
    }
    fn reload(&mut self, change: Option<Change>, cx: &mut Context<Self>) {
        let selection_id = match &change {
            Some(Change::Theme {
                after: Some(theme), ..
            }) => Some(theme.id.clone()),
            _ => self.selected.as_ref().map(|theme| theme.id.clone()),
        };
        let root = self.root.clone();
        self.busy = true;
        self.task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    match change {
                        Some(change) => native_preferences::save(&root, change),
                        None => native_preferences::load(&root),
                    }
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                match result {
                    Ok(settings) => {
                        install(&settings);
                        this.themes = settings.custom_themes;
                        this.selected = selection_id.and_then(|id| {
                            this.themes.iter().find(|theme| theme.id == id).cloned()
                        });
                        this.draft = None;
                        this.error = None;
                        let selection = crate::appearance::themes(cx);
                        for (mode, id) in [
                            (crate::theme::Appearance::Light, selection.light.clone()),
                            (crate::theme::Appearance::Dark, selection.dark.clone()),
                        ] {
                            let id = if zeron_theme::ThemeRegistry::active().variant(&id).is_some()
                            {
                                id
                            } else {
                                match mode {
                                    crate::theme::Appearance::Light => "kivio-neutral-light",
                                    crate::theme::Appearance::Dark => "kivio-neutral-dark",
                                }
                                .into()
                            };
                            crate::appearance::set_theme(mode, id, cx);
                        }
                        crate::appearance::apply_registry_change(cx);
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }
    fn current(&self, cx: &App) -> ThemeDefinition {
        if let Some(theme) = &self.selected {
            return theme.clone();
        }
        let selection = crate::appearance::themes(cx);
        if let Some(theme) = self
            .themes
            .iter()
            .find(|theme| selection.light == format!("kivio-{}-light", theme.id))
        {
            return theme.clone();
        }
        let builtins = builtin_themes();
        builtins
            .iter()
            .find(|theme| selection.light == format!("kivio-{}-light", theme.id))
            .unwrap_or(&builtins[0])
            .clone()
    }
    fn capture(&self, cx: &App) -> Option<Value> {
        let mut value = self.draft.clone()?;
        value["name"] = json!(self.name.read(cx).value().to_string());
        for (key, input) in &self.colors {
            value[self.mode][*key] = json!(input.read(cx).value().to_string());
        }
        Some(value)
    }
    fn show_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(draft) = &self.draft {
            for (key, input) in &self.colors {
                let value = draft[self.mode][*key].as_str().unwrap_or("").to_owned();
                input.update(cx, |input, cx| input.set_value(value, window, cx));
            }
        }
    }
    fn edit(&mut self, copy: bool, window: &mut Window, cx: &mut Context<Self>) {
        let mut theme = self.current(cx);
        self.baseline = if copy { None } else { Some(theme.clone()) };
        if copy {
            theme.id = uuid::Uuid::new_v4().to_string();
            theme.name = crate::ui_format!("{} 副本", "{} copy", theme.name);
        }
        self.name.update(cx, |input, cx| {
            input.set_value(theme.name.clone(), window, cx)
        });
        self.draft = serde_json::to_value(theme).ok();
        self.mode = "light";
        self.show_palette(window, cx);
        self.error = None;
        cx.notify();
    }
    fn apply(&mut self, theme: ThemeDefinition, cx: &mut Context<Self>) {
        let root = self.root.clone();
        self.busy = true;
        self.task = Some(cx.spawn(async move |this, cx| {
            let id = theme.id.clone();
            let result = cx
                .background_spawn(async move {
                    let settings = native_preferences::load(&root)?;
                    native_preferences::save(
                        &root,
                        Change::Fields(vec![native_preferences::FieldEdit {
                            path: "/themeColor".into(),
                            before: json!(settings.theme_color),
                            after: json!(id),
                        }]),
                    )
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                match result {
                    Ok(settings) => {
                        install(&settings);
                        for (mode, suffix) in [
                            (crate::theme::Appearance::Light, "light"),
                            (crate::theme::Appearance::Dark, "dark"),
                        ] {
                            crate::appearance::set_theme(
                                mode,
                                format!("kivio-{}-{suffix}", theme.id),
                                cx,
                            );
                        }
                        crate::appearance::set_accent(
                            zeron_theme::AccentSelection::ThemeDefault,
                            cx,
                        );
                        this.error = None;
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }
    fn import(&mut self, cx: &mut Context<Self>) {
        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("导入 Kivio 主题 JSON".into()),
        });
        let root = self.root.clone();
        self.busy = true;
        self.task = Some(cx.spawn(async move |this, cx| {
            let result = async {
                let Some(paths) = prompt
                    .await
                    .map_err(|e| e.to_string())?
                    .map_err(|e| e.to_string())?
                else {
                    return Ok(None);
                };
                let Some(path) = paths.into_iter().next() else {
                    return Ok(None);
                };
                cx.background_spawn(async move {
                    let metadata = std::fs::metadata(&path).map_err(|e| e.to_string())?;
                    if metadata.len() > 1024 * 1024 {
                        return Err("主题文件过大".into());
                    }
                    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
                    let theme = kivio_model_runtime::native_themes::parse(&text)?;
                    native_preferences::save(
                        &root,
                        Change::Theme {
                            before: None,
                            after: Some(theme),
                        },
                    )
                    .map(Some)
                })
                .await
            }
            .await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                match result {
                    Ok(Some(settings)) => {
                        install(&settings);
                        this.themes = settings.custom_themes;
                        this.error = None;
                    }
                    Ok(None) => {}
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }
    fn export(&mut self, cx: &mut Context<Self>) {
        let theme = self.current(cx);
        let prompt = cx.prompt_for_new_path(&self.root, Some("Kivio-theme.json"));
        self.busy = true;
        self.task = Some(cx.spawn(async move |this, cx| {
            let result = async {
                let Some(path) = prompt
                    .await
                    .map_err(|e| e.to_string())?
                    .map_err(|e| e.to_string())?
                else {
                    return Ok(());
                };
                cx.background_spawn(async move {
                    kivio_model_runtime::native_themes::export(&theme, &path)
                })
                .await
            }
            .await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                this.error = result.err();
                cx.notify();
            });
        }));
        cx.notify();
    }
}
impl Render for CustomThemes {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let english = crate::i18n::is_english();
        if self.english_placeholders != english {
            self.english_placeholders = english;
            self.name.update(cx, |input, cx| input.set_placeholder(crate::i18n::text("主题名称"), window, cx));
        }
        let mut body = div().v_flex().gap_4().child(crate::i18n::text("自定义主题"));
        let current = self.current(cx);
        let editable = self.themes.iter().any(|theme| theme.id == current.id);
        body = body.child(
            div()
                .flex()
                .flex_wrap()
                .gap_2()
                .child(
                    Button::new("theme-copy")
                        .small()
                        .label(crate::i18n::text("复制当前主题"))
                        .disabled(self.busy || self.draft.is_some())
                        .on_click(cx.listener(|this, _, window, cx| this.edit(true, window, cx))),
                )
                .child(
                    Button::new("theme-import")
                        .small()
                        .label(crate::i18n::text("导入 JSON…"))
                        .disabled(self.busy || self.draft.is_some())
                        .on_click(cx.listener(|this, _, _, cx| this.import(cx))),
                )
                .child(
                    Button::new("theme-export")
                        .small()
                        .label(crate::i18n::text("导出 JSON…"))
                        .disabled(self.busy || self.draft.is_some())
                        .on_click(cx.listener(|this, _, _, cx| this.export(cx))),
                ),
        );
        for theme in &self.themes {
            let choose = theme.clone();
            let apply = theme.clone();
            body = body.child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        Button::new(SharedString::from(format!("select-theme-{}", theme.id)))
                            .small()
                            .label(theme.name.clone())
                            .selected(current.id == theme.id)
                            .disabled(self.busy || self.draft.is_some())
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.selected = Some(choose.clone());
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new(SharedString::from(format!("apply-theme-{}", theme.id)))
                            .small()
                            .label(crate::i18n::text("应用"))
                            .disabled(self.busy || self.draft.is_some())
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.apply(apply.clone(), cx)),
                            ),
                    ),
            );
        }
        if editable && self.draft.is_none() {
            body = body.child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        Button::new("edit-custom-theme")
                            .small()
                            .label(crate::i18n::text("编辑所选主题"))
                            .disabled(self.busy)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.edit(false, window, cx)),
                            ),
                    )
                    .child(
                        Button::new("delete-custom-theme")
                            .small()
                            .label(crate::i18n::text("删除…"))
                            .disabled(self.busy)
                            .on_click(cx.listener(|this, _, window, cx| {
                                let before = this.current(cx);
                                let owner = cx.weak_entity();
                                window.open_dialog(cx, move |dialog, _, _| {
                                    let owner = owner.clone();
                                    let before = before.clone();
                                    dialog
                                        .title(crate::i18n::text("删除主题"))
                                        .child(crate::ui_format!("删除“{}”？正在使用此主题时将回到中性主题。", "Delete “{}”? If this theme is active, the neutral theme will be applied.",
                                            before.name
                                        ))
                                        .footer(
                                            div()
                                                .flex()
                                                .gap_2()
                                                .child(
                                                    Button::new("cancel-delete-theme")
                                                        .small()
                                                        .label(crate::i18n::text("取消"))
                                                        .on_click(|_, window, cx| {
                                                            window.close_dialog(cx)
                                                        }),
                                                )
                                                .child(
                                                    Button::new("confirm-delete-theme")
                                                        .small()
                                                        .label(crate::i18n::text("删除"))
                                                        .on_click(move |_, window, cx| {
                                                            let _ = owner.update(cx, |this, cx| {
                                                                this.selected = None;
                                                                this.reload(
                                                                    Some(Change::Theme {
                                                                        before: Some(
                                                                            before.clone(),
                                                                        ),
                                                                        after: None,
                                                                    }),
                                                                    cx,
                                                                )
                                                            });
                                                            window.close_dialog(cx);
                                                        }),
                                                ),
                                        )
                                });
                            })),
                    ),
            );
        }
        if let Some(draft) = self.capture(cx) {
            body = body.child(Input::new(&self.name).small().disabled(self.busy));
            let mut modes = div().flex().gap_2();
            for (mode, label) in [("light", "浅色色板"), ("dark", "深色色板")] {
                modes = modes.child(
                    Button::new(mode)
                        .small()
                        .label(crate::i18n::display(label))
                        .selected(self.mode == mode)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.draft = this.capture(cx);
                            this.mode = mode;
                            this.show_palette(window, cx);
                            cx.notify();
                        })),
                );
            }
            body = body.child(modes);
            if let Ok(family) = zeron_theme::kivio_family(&draft) {
                body = body.child(
                    div().flex().h(px(100.)).children(
                        family
                            .variants
                            .iter()
                            .map(crate::kivio_settings::theme_miniature),
                    ),
                );
            }
            let mut fields = div().flex().flex_wrap().gap_3();
            for (key, label) in COLORS {
                fields = fields.child(
                    div().w(px(190.)).v_flex().gap_1().child(*label).child(
                        Input::new(&self.colors[key])
                            .small()
                            .aria_label(*label)
                            .disabled(self.busy),
                    ),
                );
            }
            body = body.child(fields);
            body = body.child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        Button::new("save-theme")
                            .small()
                            .label(crate::i18n::text("保存主题"))
                            .disabled(self.busy)
                            .on_click(cx.listener(|this, _, _, cx| {
                                match this
                                    .capture(cx)
                                    .and_then(|draft| serde_json::from_value(draft).ok())
                                {
                                    Some(after) => this.reload(
                                        Some(Change::Theme {
                                            before: this.baseline.clone(),
                                            after: Some(after),
                                        }),
                                        cx,
                                    ),
                                    None => {
                                        this.error = Some("主题格式无效".into());
                                        cx.notify();
                                    }
                                }
                            })),
                    )
                    .child(
                        Button::new("cancel-theme")
                            .small()
                            .label(crate::i18n::text("放弃编辑"))
                            .disabled(self.busy)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.draft = None;
                                this.error = None;
                                cx.notify();
                            })),
                    ),
            );
        }
        if let Some(error) = &self.error {
            body = body.child(
                Button::new("reload-theme")
                    .small()
                    .label(crate::i18n::text("放弃编辑并重新读取"))
                    .disabled(self.busy)
                    .on_click(cx.listener(|this, _, _, cx| this.reload(None, cx))),
            );
            body = body.child(
                div()
                    .text_sm()
                    .text_color(cx.theme().danger)
                    .child(error.clone()),
            );
        }
        body
    }
}

fn builtin_themes() -> &'static [ThemeDefinition] {
    static THEMES: std::sync::OnceLock<Vec<ThemeDefinition>> = std::sync::OnceLock::new();
    THEMES.get_or_init(|| serde_json::from_str(include_str!("../../../../src/theme/builtins.json")).expect("built-in palettes"))
}
