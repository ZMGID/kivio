use super::*;
use gpui_kit::component::Sizable;
use kivio_model_runtime::tool_settings::SubAgentModelSelection;
impl SettingsPage {
    pub(super) fn mixer(&self, theme: &Theme, cx: &Context<Self>) -> Div {
        let settings = self.settings.as_ref().unwrap();
        let mut roles = Vec::new();
        let choices = self.providers.read(cx).models();
        for (role, label, inherit) in [
            ("task", "TASK · 通用与编码", "跟随主对话"),
            ("smol", "SMOL · 只读研究", "跟随 TASK"),
            ("slow", "SLOW · 审查与分析", "跟随 TASK"),
        ] {
            let current = settings.chat_tools.sub_agent_models.get(role).cloned();
            let selected = current.clone().unwrap_or_default();
            let title = choices
                .iter()
                .find(|choice| {
                    choice.provider_id == selected.provider_id && choice.model == selected.model
                })
                .map(|choice| choice.label.to_string())
                .unwrap_or_else(|| {
                    if selected.model.is_empty() {
                        inherit.to_string()
                    } else {
                        format!("{} · {}", selected.provider_id, selected.model)
                    }
                });
            let mut efforts = vec![(None, "模型设置".to_string())];
            let mut adjustable = false;
            if !selected.provider_id.is_empty() && !selected.model.is_empty() {
                if let Ok((provider, model)) = self
                    .providers
                    .read(cx)
                    .selected(&selected.provider_id, &selected.model)
                {
                    use kivio_model_runtime::model_metadata::{
                        ThinkingOffMode, thinking_capabilities_for_model,
                    };
                    let capabilities = thinking_capabilities_for_model(Some(&provider), &model);
                    if !matches!(
                        capabilities.off_mode,
                        ThinkingOffMode::Unsupported | ThinkingOffMode::NotApplicable
                    ) {
                        efforts.push((
                            Some("off".to_string()),
                            if capabilities.off_mode == ThinkingOffMode::UpfrontOnly {
                                "关闭前置思考"
                            } else {
                                "关闭思考"
                            }
                            .into(),
                        ));
                    }
                    efforts.extend(
                        capabilities
                            .levels
                            .into_iter()
                            .map(|level| (Some(level.clone()), level)),
                    );
                    adjustable = efforts.len() > 1 || selected.thinking_level.is_some();
                }
            }
            let effort_owner = cx.weak_entity();
            let effort_before = current.clone();
            let effort_selection = selected.clone();
            let effort_label = selected
                .thinking_level
                .clone()
                .unwrap_or_else(|| "模型设置".into());
            let effort_button = Button::new(SharedString::from(format!("{role}-thinking")))
                .small()
                .label(effort_label)
                .dropdown_caret(true)
                .disabled(self.busy || !adjustable)
                .dropdown_menu(move |mut menu, _, _| {
                    for (value, label) in &efforts {
                        let owner = effort_owner.clone();
                        let before = effort_before.clone();
                        let mut after = effort_selection.clone();
                        after.thinking_level = value.clone();
                        menu = menu.item(
                            PopupMenuItem::new(label.clone())
                                .checked(effort_selection.thinking_level == *value)
                                .on_click(move |_, _, cx| {
                                    let _ = owner.update(cx, |this, cx| {
                                        this.save(
                                            Change::SubAgent {
                                                role: role.into(),
                                                before: before.clone(),
                                                after: after.clone(),
                                            },
                                            cx,
                                        )
                                    });
                                }),
                        );
                    }
                    menu
                });
            let owner = cx.weak_entity();
            let choices = choices.clone();
            roles.push(row(
                label,
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .child(
                        Button::new(role)
                            .small()
                            .label(title)
                            .dropdown_caret(true)
                            .disabled(self.busy)
                            .dropdown_menu(move |mut menu, _, _| {
                                let reset = owner.clone();
                                let baseline = current.clone();
                                menu = menu.item(
                                    PopupMenuItem::new(inherit)
                                        .checked(selected.model.is_empty())
                                        .on_click(move |_, _, cx| {
                                            let _ = reset.update(cx, |this, cx| {
                                                this.save(
                                                    Change::SubAgent {
                                                        role: role.into(),
                                                        before: baseline.clone(),
                                                        after: SubAgentModelSelection::default(),
                                                    },
                                                    cx,
                                                )
                                            });
                                        }),
                                );
                                for choice in &choices {
                                    let owner = owner.clone();
                                    let before = current.clone();
                                    let choice = choice.clone();
                                    menu = menu.item(
                                        PopupMenuItem::new(choice.label.clone())
                                            .checked(
                                                choice.provider_id == selected.provider_id
                                                    && choice.model == selected.model,
                                            )
                                            .on_click(move |_, _, cx| {
                                                let _ = owner.update(cx, |this, cx| {
                                                    this.save(
                                                        Change::SubAgent {
                                                            role: role.into(),
                                                            before: before.clone(),
                                                            after: SubAgentModelSelection {
                                                                provider_id: choice
                                                                    .provider_id
                                                                    .clone(),
                                                                model: choice.model.clone(),
                                                                thinking_level: None,
                                                            },
                                                        },
                                                        cx,
                                                    )
                                                });
                                            }),
                                    );
                                }
                                menu
                            }),
                    )
                    .child(effort_button)
                    .into_any_element(),
                theme,
            ));
        }
        div()
            .flex()
            .flex_col()
            .gap_5()
            .child(self.providers.clone())
            .child(self.prompt_editor(
                TextPreference::PromptOptimize,
                "提问优化提示词",
                "留空使用内置模板；只影响提问优化，不改变主对话提示词。",
                theme,
                cx,
            ))
            .child(card("子代理", roles, theme))
            .child(self.text_field(
                "子代理并发上限",
                "/chatTools/subAgentConcurrency",
                "1–64。已有子代理保留启动时的配置。",
                theme,
                cx,
            ))
            .child(
                div()
                    .text_sm()
                    .text_color(theme.text_muted)
                    .child(crate::i18n::text("模型用途与角色配置已保存；原生子代理执行尚未接入。")),
            )
    }
}
