//! Original usage summary and call log backed by Kivio's durable ledger.
use gpui_kit::component::{
    ActiveTheme, Disableable, Selectable, StyledExt, button::Button, chart::BarChart,
};
use gpui_kit::{prelude::*, *};
use kivio_model_runtime::usage_ledger::{self, UsageStatsQuery, UsageStatsResponse};

pub struct UsagePage {
    directory: std::path::PathBuf,
    range: &'static str,
    logs: bool,
    snapshot: Option<UsageStatsResponse>,
    error: Option<String>,
    busy: bool,
    task: Option<Task<()>>,
}
impl UsagePage {
    pub fn new(directory: std::path::PathBuf, cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            directory,
            range: "7d",
            logs: false,
            snapshot: None,
            error: None,
            busy: false,
            task: None,
        };
        this.refresh(cx);
        this
    }
    fn refresh(&mut self, cx: &mut Context<Self>) {
        let root = self.directory.clone();
        let range = self.range.to_string();
        self.busy = true;
        self.task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let mut dirs = vec![root.join("usage")];
                    match std::fs::read_dir(root.join("conversations")) {
                        Ok(entries) => {
                            for entry in entries {
                                let entry = entry.map_err(|e| e.to_string())?;
                                if entry.file_type().map_err(|e| e.to_string())?.is_dir() {
                                    dirs.push(entry.path().join("usage"));
                                }
                            }
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                        Err(error) => return Err(error.to_string()),
                    }
                    Ok(usage_ledger::stats_in_directories(
                        &dirs,
                        UsageStatsQuery {
                            range,
                            limit: Some(100),
                            ..Default::default()
                        },
                    ))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                match result {
                    Ok(snapshot) => {
                        this.snapshot = Some(snapshot);
                        this.error = None;
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }
}
impl Render for UsagePage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut body = div().v_flex().gap_4();
        let mut toolbar = div().flex().flex_wrap().items_center().gap_2();
        for (logs, label) in [(false, "应用用量"), (true, "调用明细")] {
            toolbar = toolbar.child(
                Button::new(label)
                    .label(label)
                    .selected(self.logs == logs)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.logs = logs;
                        cx.notify();
                    })),
            );
        }
        toolbar = toolbar.child(div().flex_1()).child(
            Button::new("usage-refresh")
                .label(if self.busy { "读取中…" } else { "刷新" })
                .disabled(self.busy)
                .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))),
        );
        body = body.child(toolbar);
        let mut ranges = div().flex().gap_2();
        for (range, label) in [
            ("today", "当天"),
            ("1d", "近 1 日"),
            ("7d", "近 7 日"),
            ("30d", "近 30 日"),
        ] {
            ranges = ranges.child(
                Button::new(range)
                    .label(label)
                    .selected(self.range == range)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.range = range;
                        this.refresh(cx);
                    })),
            );
        }
        body = body.child(ranges);
        if let Some(error) = &self.error {
            body = body.child(div().text_color(cx.theme().danger).child(error.clone()));
        }
        if let Some(stats) = &self.snapshot {
            if !self.logs {
                let summary = &stats.summary;
                let hit = if summary.input_tokens == 0 {
                    0.
                } else {
                    100. * summary.cached_input_tokens as f64 / summary.input_tokens as f64
                };
                let mut summary_row = div().flex().flex_wrap().gap_3();
                for (title, value, detail) in [
                    (
                        "累计 Token",
                        summary.total_tokens.to_string(),
                        format!("{} 次请求", summary.total_requests),
                    ),
                    (
                        "成本",
                        format!("${:.4}", summary.total_cost_usd),
                        "USD".into(),
                    ),
                    ("缓存命中", format!("{hit:.1}%"), "占输入 Token".into()),
                    (
                        "平均耗时",
                        summary
                            .average_duration_ms
                            .map(|ms| format!("{:.1} s", ms / 1000.))
                            .unwrap_or_else(|| "—".into()),
                        "已记录的请求".into(),
                    ),
                ] {
                    summary_row = summary_row.child(
                        div()
                            .flex_1()
                            .min_w(px(130.))
                            .border_1()
                            .border_color(cx.theme().border)
                            .rounded_lg()
                            .p_4()
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(title),
                            )
                            .child(
                                div()
                                    .my_2()
                                    .text_size(px(22.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(value),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(detail),
                            ),
                    );
                }
                body = body.child(summary_row).child(
                    div()
                        .v_flex()
                        .gap_3()
                        .border_1()
                        .border_color(cx.theme().border)
                        .rounded_lg()
                        .p_4()
                        .child("Token 趋势")
                        .child(
                            div().h(px(220.)).child(
                                BarChart::new(stats.trend.clone())
                                    .id("usage-token-trend")
                                    .band(|point| point.label.clone())
                                    .value(|point| point.total_tokens as f64)
                                    .value_axis(true),
                            ),
                        ),
                );
            } else {
                let mut rows = div().v_flex().gap_2();
                for record in &stats.logs {
                    rows = rows.child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .py_3()
                            .border_b_1()
                            .border_color(cx.theme().border)
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .child(format!("{} · {}", record.provider_name, record.model))
                                    .child(
                                        div()
                                            .text_sm()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(format!(
                                                "{} · {}",
                                                record.source, record.operation
                                            )),
                                    ),
                            )
                            .child(record.status.clone())
                            .child(format!("{} tokens", record.total_tokens.unwrap_or(0)))
                            .child(format!("{:.1} s", record.duration_ms as f64 / 1000.)),
                    );
                }
                if stats.logs.is_empty() {
                    rows = rows.child("当前范围内没有调用记录。");
                }
                body = body.child(rows).child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(format!(
                            "共 {} 条调用，显示最近 {} 条。",
                            stats.total_logs,
                            stats.logs.len()
                        )),
                );
            }
        }
        crate::kivio_settings::page_frame(
            "使用统计",
            "模型调用、Token 用量与成本。",
            body.into_any_element(),
            cx,
        )
    }
}
