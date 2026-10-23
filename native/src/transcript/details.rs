// Fork divider adapted from zeron/crates/ui/src/transcript.rs (fork_marker).
// Copyright (c) 2026 Wing. MIT; see native/vendor/zeron-ui-LICENSE.
// Tool disclosure follows the same grouped/closed-body-skip interaction, with
// Kivio records as the authority and Kit buttons/theme replacing Zeron widgets.
use super::*;
use crate::conversation_data::{ToolCallRecord, ToolCallStatus};

pub(crate) fn fork_marker(source_title: SharedString, cx: &App) -> AnyElement {
    let rule = || div().flex_1().min_w_0().h(px(1.)).bg(cx.theme().border);
    div()
        .py_3()
        .w_full()
        .min_w_0()
        .overflow_hidden()
        .v_flex()
        .gap_1()
        .child(
            div()
                .w_full()
                .min_w_0()
                .flex()
                .items_center()
                .gap_2()
                .child(rule())
                .child(
                    div()
                        .flex_none()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child("分支来源"),
                )
                .child(rule()),
        )
        .child(
            div()
                .w_full()
                .min_w_0()
                .overflow_hidden()
                .text_ellipsis()
                .text_center()
                .text_sm()
                .font_weight(FontWeight::MEDIUM)
                .text_color(cx.theme().muted_foreground)
                .child(source_title),
        )
        .into_any_element()
}

fn literal(id: &'static str, value: String) -> TextView {
    let escaped = value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('\n', "<br>");
    TextView::html(id, format!("<p>{escaped}</p>")).selectable(true)
}

fn status(tool: &ToolCallRecord) -> &'static str {
    match tool.status {
        ToolCallStatus::Pending => "等待中",
        ToolCallStatus::Running => "运行中",
        ToolCallStatus::Success => "已完成",
        ToolCallStatus::Error => "失败",
        ToolCallStatus::Cancelled => "已取消",
        ToolCallStatus::Skipped => "已跳过",
    }
}

impl Transcript {
    pub(super) fn tool_details(&self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let message = &self.messages[index];
        let tools = &message.record.tool_calls;
        let id = message.record.id.clone();
        let running = tools.iter().any(|tool| {
            matches!(
                tool.status,
                ToolCallStatus::Pending | ToolCallStatus::Running
            )
        });
        let failed = tools
            .iter()
            .any(|tool| tool.status == ToolCallStatus::Error);
        let open = self
            .tool_folds
            .get(&id)
            .copied()
            .unwrap_or(running || failed);
        let toggle_id = id.clone();
        let mut group = div().v_flex().w_full().min_w_0().gap_1().child(
            Button::new(SharedString::from(format!("tool-group-{id}")))
                .ghost()
                .small()
                .icon(if open {
                    IconName::ChevronDown
                } else {
                    IconName::ChevronRight
                })
                .label(format!(
                    "{} · {} 项工具调用",
                    if running {
                        "正在工作"
                    } else if failed {
                        "部分调用失败"
                    } else {
                        "执行记录"
                    },
                    tools.len()
                ))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.tool_folds.insert(toggle_id.clone(), !open);
                    this.scroller
                        .update(cx, |state, cx| state.remeasure_items(index..index + 1, cx));
                    cx.notify();
                })),
        );
        // Do not build hidden argument/output trees for collapsed groups.
        if open {
            for tool in tools {
                let key = SharedString::from(format!("tool-{id}-{}", tool.id));
                group = group.child(
                    div()
                        .v_flex()
                        .min_w_0()
                        .ml_3()
                        .pl_3()
                        .py_2()
                        .border_l_1()
                        .border_color(cx.theme().border)
                        .gap_1()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(
                                    div()
                                        .text_sm()
                                        .font_weight(FontWeight::MEDIUM)
                                        .child(tool.name.clone()),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(if tool.status == ToolCallStatus::Error {
                                            cx.theme().danger
                                        } else {
                                            cx.theme().muted_foreground
                                        })
                                        .child(status(tool)),
                                )
                                .when_some(tool.duration_ms, |row, ms| {
                                    row.child(
                                        div()
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(format!("{ms} ms")),
                                    )
                                }),
                        )
                        .child(
                            Button::new(key)
                                .small()
                                .ghost()
                                .label("查看参数与结果")
                                .on_click({
                                    let tool = tool.clone();
                                    move |_, window, cx| {
                                        let tool = tool.clone();
                                        window.open_dialog(cx, move |dialog, _, _| {
                                            dialog.title(tool.name.clone()).width(px(672.)).child(
                                                div()
                                                    .id("tool-detail-scroll")
                                                    .v_flex()
                                                    .gap_3()
                                                    .max_h(rems(32.))
                                                    .overflow_y_scroll()
                                                    .child(
                                                        div()
                                                            .text_sm()
                                                            .font_weight(FontWeight::MEDIUM)
                                                            .child("参数"),
                                                    )
                                                    .child(literal(
                                                        "tool-arguments",
                                                        tool.arguments.clone(),
                                                    ))
                                                    .child(
                                                        div()
                                                            .text_sm()
                                                            .font_weight(FontWeight::MEDIUM)
                                                            .child("结果"),
                                                    )
                                                    .child(literal(
                                                        "tool-result",
                                                        tool.error
                                                            .clone()
                                                            .or_else(|| tool.result_preview.clone())
                                                            .unwrap_or_else(|| "暂无结果".into()),
                                                    )),
                                            )
                                        });
                                    }
                                }),
                        ),
                );
            }
        }
        group.into_any_element()
    }
}
