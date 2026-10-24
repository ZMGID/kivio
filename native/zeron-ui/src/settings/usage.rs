//! Original usage summary and call log backed by Kivio's durable ledger.
use chrono::{Datelike, NaiveDate};
use gpui_kit::component::Sizable;
use gpui_kit::component::{
    ActiveTheme, Disableable, Selectable, StyledExt, WindowExt,
    button::Button,
    chart::BarChart,
    input::{Input, InputState},
    menu::{DropdownMenu, PopupMenuItem},
};
use gpui_kit::{prelude::*, *};
use kivio_model_runtime::usage_ledger::{self, UsageStatsQuery, UsageStatsResponse};

#[derive(Clone, Copy, PartialEq, Eq)]
enum ActivityMode {
    Day,
    Week,
    Total,
}

struct ActivityDay {
    date: NaiveDate,
    tokens: u64,
    requests: u64,
    week_tokens: u64,
    cumulative_tokens: u64,
}
impl ActivityDay {
    fn value(&self, mode: ActivityMode) -> u64 {
        match mode {
            ActivityMode::Day => self.tokens,
            ActivityMode::Week => self.week_tokens,
            ActivityMode::Total => self.cumulative_tokens,
        }
    }
    fn detail(&self, mode: ActivityMode) -> String {
        let day = crate::ui_format!("{} · {} 次请求 · {} tokens", "{} · {} requests · {} tokens",
            self.date, self.requests, self.tokens
        );
        match mode {
            ActivityMode::Day => day,
            ActivityMode::Week => crate::ui_format!("{day} · 本周 {} tokens", "{day} · This week: {} tokens", self.week_tokens),
            ActivityMode::Total => crate::ui_format!("{day} · 近一年累计 {} tokens", "{day} · Year-to-date total: {} tokens", self.cumulative_tokens),
        }
    }
}

fn activity_days(points: &[usage_ledger::UsageTrendPoint], today: NaiveDate) -> Vec<ActivityDay> {
    let mut by_date = std::collections::BTreeMap::<NaiveDate, (u64, u64)>::new();
    for point in points {
        if let Some(date) = point
            .date
            .get(..10)
            .and_then(|date| NaiveDate::parse_from_str(date, "%Y-%m-%d").ok())
        {
            let day = by_date.entry(date).or_default();
            day.0 = day.0.saturating_add(point.total_tokens);
            day.1 = day.1.saturating_add(point.requests);
        }
    }
    let mut cumulative = 0_u64;
    let mut weeks = std::collections::BTreeMap::<NaiveDate, u64>::new();
    let mut days: Vec<_> = (0..365)
        .rev()
        .map(|offset| {
            let date = today - chrono::Duration::days(offset);
            let (tokens, requests) = by_date.get(&date).copied().unwrap_or_default();
            cumulative = cumulative.saturating_add(tokens);
            let monday =
                date - chrono::Duration::days(date.weekday().num_days_from_monday() as i64);
            let week = weeks.entry(monday).or_default();
            *week = week.saturating_add(tokens);
            ActivityDay {
                date,
                tokens,
                requests,
                week_tokens: 0,
                cumulative_tokens: cumulative,
            }
        })
        .collect();
    for day in &mut days {
        let monday =
            day.date - chrono::Duration::days(day.date.weekday().num_days_from_monday() as i64);
        day.week_tokens = weeks[&monday];
    }
    days
}

pub struct UsagePage {
    english_placeholders: bool,
    directory: std::path::PathBuf,
    range: &'static str,
    source: &'static str,
    status: &'static str,
    provider: Entity<InputState>,
    model: Entity<InputState>,
    offset: usize,
    logs: bool,
    group: Option<&'static str>,
    expanded: Option<String>,
    debug: bool,
    debug_enabled: bool,
    records: Vec<kivio_model_runtime::request_debug::RequestDebugRecord>,
    snapshot: Option<UsageStatsResponse>,
    error: Option<String>,
    busy: bool,
    task: Option<Task<()>>,
    activity: Vec<ActivityDay>,
    activity_mode: ActivityMode,
    activity_selected: Option<NaiveDate>,
    activity_busy: bool,
    activity_error: Option<String>,
    activity_skipped: usize,
    activity_task: Option<Task<()>>,
}
impl UsagePage {
    pub fn new(directory: std::path::PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            english_placeholders: crate::i18n::is_english(),
            directory,
            range: "7d",
            source: "all",
            status: "all",
            offset: 0,
            provider: cx.new(|cx| InputState::new(window, cx).placeholder(crate::i18n::text("搜索供应商"))),
            model: cx.new(|cx| InputState::new(window, cx).placeholder(crate::i18n::text("搜索模型"))),
            logs: false,
            group: None,
            expanded: None,
            debug: false,
            debug_enabled: false,
            records: vec![],
            snapshot: None,
            error: None,
            busy: false,
            task: None,
            activity: Vec::new(),
            activity_mode: ActivityMode::Day,
            activity_selected: None,
            activity_busy: false,
            activity_error: None,
            activity_skipped: 0,
            activity_task: None,
        };
        this.refresh(cx);
        this.refresh_activity(cx);
        this
    }
    fn refresh_activity(&mut self, cx: &mut Context<Self>) {
        let root = self.directory.clone();
        self.activity_busy = true;
        self.activity_error = None;
        // This task deliberately owns an unfiltered query, independent of the
        // selected summary range, source, status, provider and model.
        self.activity_task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let stats = usage_ledger::stats_in_directories(
                        &directories(&root)?,
                        UsageStatsQuery {
                            range: "365d".into(),
                            limit: Some(0),
                            ..Default::default()
                        },
                    );
                    let days = activity_days(&stats.trend, chrono::Local::now().date_naive());
                    Ok::<_, String>((days, stats.skipped_records))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.activity_busy = false;
                match result {
                    Ok((days, skipped)) => {
                        this.activity = days;
                        this.activity_skipped = skipped;
                    }
                    Err(error) => this.activity_error = Some(error),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn render_activity(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut modes = div().flex().gap_2();
        for (mode, label) in [
            (ActivityMode::Day, "每日"),
            (ActivityMode::Week, "每周"),
            (ActivityMode::Total, "累计"),
        ] {
            modes = modes.child(
                Button::new(label)
                    .small()
                    .label(crate::i18n::display(label))
                    .selected(self.activity_mode == mode)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.activity_mode = mode;
                        cx.notify();
                    })),
            );
        }
        let mut card = div()
            .v_flex()
            .gap_3()
            .p_4()
            .rounded(px(12.))
            .bg(cx.theme().group_box)
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(crate::i18n::text("Token 活动"))
                    .child(modes),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(crate::i18n::text("近一年 · 全部来源与模型，不受下方筛选影响")),
            );
        if self.activity_busy {
            card = card.child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(crate::i18n::text("正在读取年度活动…")),
            );
        }
        if let Some(error) = &self.activity_error {
            card = card.child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().danger)
                            .child(crate::ui_format!("年度活动读取失败：{error}", "Could not load annual activity: {error}")),
                    )
                    .child(
                        Button::new("retry-usage-activity")
                            .small()
                            .label(crate::i18n::text("重试"))
                            .disabled(self.activity_busy)
                            .on_click(cx.listener(|this, _, _, cx| this.refresh_activity(cx))),
                    ),
            );
        }
        if self.activity.is_empty() {
            return card.into_any_element();
        }
        let max = self
            .activity
            .iter()
            .map(|day| day.value(self.activity_mode))
            .max()
            .unwrap_or(0);
        let mut columns = div().flex().gap(px(3.));
        let mut months = div()
            .whitespace_nowrap()
            .flex()
            .gap(px(3.))
            .text_xs()
            .text_color(cx.theme().muted_foreground);
        let first = self.activity[0].date;
        let start = first - chrono::Duration::days(first.weekday().num_days_from_monday() as i64);
        let last = self.activity.last().unwrap().date;
        let selected = self.activity_selected.unwrap_or(last);
        let mut week = start;
        let mut previous_month = None;
        while week <= last {
            let mut column = div().v_flex().gap(px(3.)).w(px(12.)).flex_shrink_0();
            for offset in 0..7 {
                let date = week + chrono::Duration::days(offset);
                let index = (date - first).num_days();
                if index < 0 || date > last {
                    column = column.child(div().size(px(12.)));
                    continue;
                }
                let day = &self.activity[index as usize];
                let value = day.value(self.activity_mode);
                let level = if value == 0 || max == 0 {
                    0
                } else {
                    ((value as f64 / max as f64 * 4.).ceil() as u8).clamp(1, 4)
                };
                let color = if level == 0 {
                    cx.theme().muted
                } else {
                    cx.theme().primary.opacity(0.25 + level as f32 * 0.1875)
                };
                let detail = day.detail(self.activity_mode);
                column = column.child(
                    div()
                        .id(SharedString::from(format!("usage-activity-{date}")))
                        .size(px(12.))
                        .rounded(px(3.))
                        .bg(color)
                        .border_1()
                        .border_color(if selected == date {
                            cx.theme().foreground
                        } else {
                            color
                        })
                        .tooltip(crate::settings::widgets::text_tooltip(detail))
                        .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                            if *hovered {
                                this.activity_selected = Some(date);
                                cx.notify();
                            }
                        })),
                );
            }
            let month = week.month();
            months = months.child(div().w(px(12.)).flex_shrink_0().child(
                if previous_month != Some(month) {
                    crate::ui_format!("{month}月", "Month {month}")
                } else {
                    String::new()
                },
            ));
            previous_month = Some(month);
            columns = columns.child(column);
            week += chrono::Duration::days(7);
        }
        // Horizontal overflow belongs to the calendar, not the entire settings page.
        card = card.child(
            div()
                .id("usage-activity-scroll")
                .overflow_x_scroll()
                .pb_2()
                .child(div().v_flex().gap_2().child(columns).child(months)),
        );
        let selected_day = self
            .activity
            .iter()
            .find(|day| day.date == selected)
            .unwrap_or_else(|| self.activity.last().unwrap());
        let mut navigation = div().flex().gap_2();
        for (step, label) in [(-1_i64, "上一日"), (1, "下一日")] {
            let next = selected_day.date + chrono::Duration::days(step);
            navigation = navigation.child(
                Button::new(label)
                    .small()
                    .label(crate::i18n::display(label))
                    .disabled(next < first || next > last)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.activity_selected = Some(next);
                        cx.notify();
                    })),
            );
        }
        card = card.child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .justify_between()
                .gap_2()
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(selected_day.detail(self.activity_mode)),
                )
                .child(navigation),
        );
        let mut legend = div()
            .flex()
            .items_center()
            .gap_2()
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .child(crate::i18n::text("少"));
        for level in 0..=4 {
            let color = if level == 0 {
                cx.theme().muted
            } else {
                cx.theme().primary.opacity(0.25 + level as f32 * 0.1875)
            };
            legend = legend.child(div().size(px(10.)).rounded(px(2.)).bg(color));
        }
        card = card.child(
            legend
                .child(crate::i18n::text("多"))
                .child(crate::ui_format!("· 当前视图最高 {max} tokens", "· Maximum in this view: {max} tokens")),
        );
        if self.activity.iter().all(|day| day.requests == 0) {
            card = card.child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(crate::i18n::text("近一年暂无请求记录。发送消息后刷新可查看活动。")),
            );
        }
        if self.activity_skipped > 0 {
            card = card.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(crate::ui_format!("有 {} 条账本记录未能读取，活动统计可能不完整。", "Could not read {} usage records. Activity statistics may be incomplete.",
                        self.activity_skipped
                    )),
            );
        }
        card.into_any_element()
    }

    fn clear(&mut self, debug: bool, cx: &mut Context<Self>) {
        let root = self.directory.clone();
        self.busy = true;
        self.task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    for directory in directories(&root)? {
                        let host = kivio_model_runtime::RuntimeHost::for_directory(
                            directory.parent().unwrap(),
                        );
                        if debug {
                            host.clear_debug();
                            if directory
                                .parent()
                                .unwrap()
                                .join("request_debug/records.jsonl")
                                .exists()
                            {
                                return Err("请求调试文件未能清除，请检查文件权限。".into());
                            }
                        } else {
                            host.clear_usage()?;
                        }
                    }
                    Ok::<_, String>(())
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                match result {
                    Ok(()) => {
                        this.offset = 0;
                        this.refresh(cx);
                        if !debug {
                            this.refresh_activity(cx);
                        }
                    }
                    Err(error) => {
                        this.error = Some(error);
                        cx.notify();
                    }
                }
            });
        }));
        cx.notify();
    }
    fn refresh(&mut self, cx: &mut Context<Self>) {
        let root = self.directory.clone();
        let query = UsageStatsQuery {
            range: self.range.into(),
            source: Some(self.source.into()),
            status: Some(self.status.into()),
            provider_search: Some(self.provider.read(cx).value().to_string()),
            model_search: Some(self.model.read(cx).value().to_string()),
            offset: Some(self.offset),
            limit: Some(100),
        };
        self.busy = true;
        self.task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let dirs = directories(&root)?;
                    let settings = kivio_model_runtime::native_preferences::load(&root)?;
                    let mut records = Vec::new();
                    for dir in &dirs {
                        match std::fs::read_to_string(
                            dir.parent()
                                .unwrap_or(dir)
                                .join("request_debug/records.jsonl"),
                        ) {
                            Ok(text) => {
                                for line in text.lines() {
                                    if let Ok(record) = serde_json::from_str::<
                                        kivio_model_runtime::request_debug::RequestDebugRecord,
                                    >(line)
                                    {
                                        records.push(record);
                                    }
                                }
                            }
                            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                            Err(error) => return Err(error.to_string()),
                        }
                    }
                    records.sort_by_key(|record| std::cmp::Reverse(record.created_at));
                    records.truncate(100);
                    Ok((
                        settings.chat_tools.request_debug_enabled,
                        records,
                        usage_ledger::stats_in_directories(&dirs, query),
                    ))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                match result {
                    Ok((debug_enabled, records, snapshot)) => {
                        this.debug_enabled = debug_enabled;
                        this.records = records;
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
fn directories(root: &std::path::Path) -> Result<Vec<std::path::PathBuf>, String> {
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
    Ok(dirs)
}
impl Render for UsagePage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let english = crate::i18n::is_english();
        if self.english_placeholders != english {
            self.english_placeholders = english;
            self.provider.update(cx, |input, cx| input.set_placeholder(crate::i18n::text("搜索供应商"), window, cx));
            self.model.update(cx, |input, cx| input.set_placeholder(crate::i18n::text("搜索模型"), window, cx));
        }
        let mut body = div().v_flex().gap_4();
        let mut toolbar = div().flex().flex_wrap().items_center().gap_2();
        for (logs, label) in [(false, "应用用量"), (true, "调用明细")] {
            toolbar = toolbar.child(
                Button::new(label)
                    .small()
                    .label(crate::i18n::display(label))
                    .selected(!self.debug && self.group.is_none() && self.logs == logs)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.group = None;
                        this.logs = logs;
                        this.debug = false;
                        cx.notify();
                    })),
            );
        }
        for (group, label) in [("providers", "按供应商"), ("models", "按模型")] {
            toolbar = toolbar.child(
                Button::new(group)
                    .small()
                    .label(crate::i18n::display(label))
                    .selected(!self.debug && self.group == Some(group))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.group = Some(group);
                        this.logs = false;
                        this.debug = false;
                        cx.notify();
                    })),
            );
        }
        toolbar = toolbar.child(
            Button::new("usage-debug")
                .small()
                .label(crate::i18n::text("请求调试"))
                .selected(self.debug)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.debug = true;
                    cx.notify();
                })),
        );
        toolbar = toolbar
            .child(div().flex_1())
            .child(
                Button::new("usage-clear")
                    .small()
                    .label(if self.debug { crate::i18n::text("清空调试记录…") } else { crate::i18n::text("清空用量记录…") })
                    .disabled(self.busy)
                    .on_click(cx.listener(|this, _, window, cx| {
                        let owner = cx.weak_entity();
                        let debug = this.debug;
                        window.open_dialog(cx, move |dialog, _, _| {
                            let owner = owner.clone();
                            dialog
                                .title(crate::i18n::text("清空记录"))
                                .child(if debug { crate::i18n::text("删除全部请求调试记录？此操作无法撤销。") } else { crate::i18n::text("删除全部模型用量记录？此操作不受当前筛选条件限制，无法撤销。") })
                                .footer(
                                    div()
                                        .flex()
                                        .justify_end()
                                        .gap_2()
                                        .child(
                                            Button::new("cancel-clear")
                                                .small()
                                                .label(crate::i18n::text("取消"))
                                                .on_click(|_, window, cx| window.close_dialog(cx)),
                                        )
                                        .child(
                                            Button::new("confirm-clear")
                                                .small()
                                                .label(crate::i18n::text("清空"))
                                                .on_click(move |_, window, cx| {
                                                    let _ = owner.update(cx, |this, cx| {
                                                        this.clear(debug, cx)
                                                    });
                                                    window.close_dialog(cx);
                                                }),
                                        ),
                                )
                        });
                    })),
            )
            .child(
                Button::new("usage-refresh")
                    .small()
                    .label(if self.busy { crate::i18n::text("读取中…") } else { crate::i18n::text("刷新") })
                    .disabled(self.busy)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.refresh(cx);
                        this.refresh_activity(cx);
                    })),
            );
        body = body.child(toolbar);
        if self.debug {
            body = body.child(gpui_kit::component::switch::Switch::new("request-debug-enabled").label(crate::i18n::text("记录请求调试信息")).checked(self.debug_enabled).disabled(self.busy).on_click(cx.listener(|this, enabled: &bool, _, cx| {
                let directory = this.directory.clone();
                let edit = kivio_model_runtime::native_preferences::FieldEdit { path: "/chatTools/requestDebugEnabled".into(), before: serde_json::json!(this.debug_enabled), after: serde_json::json!(*enabled) };
                this.busy = true;
                this.task = Some(cx.spawn(async move |this, cx| {
                    let result = cx.background_spawn(async move { kivio_model_runtime::native_preferences::save(&directory, kivio_model_runtime::native_preferences::Change::Fields(vec![edit])) }).await;
                    let _ = this.update(cx, |this, cx| { this.busy = false; match result { Ok(_) => this.refresh(cx), Err(error) => { this.error = Some(error); cx.notify(); } } });
                }));
                cx.notify();
            })))
            .child(div().text_sm().text_color(cx.theme().muted_foreground).child(crate::i18n::text("用于排查接口请求；开启后记录新请求，认证请求头由原有记录器脱敏。最多显示最近 100 条。")));
            if let Some(error) = &self.error {
                body = body.child(div().text_color(cx.theme().danger).child(error.clone()));
            }
            if self.records.is_empty() {
                body = body.child(crate::i18n::text("暂无请求记录。开启记录并发送一条消息后刷新。"));
            }
            for record in &self.records {
                let record = record.clone();
                body = body.child(
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
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(format!(
                                            "{} · {} ms · {}",
                                            record.operation, record.duration_ms, record.status
                                        )),
                                ),
                        )
                        .child(
                            Button::new(SharedString::from(format!("copy-debug-{}", record.id)))
                                .small()
                                .label(crate::i18n::text("复制详情"))
                                .on_click(move |_, _, cx| {
                                    if let Ok(text) = serde_json::to_string_pretty(&record) {
                                        cx.write_to_clipboard(ClipboardItem::new_string(text));
                                    }
                                }),
                        ),
                );
            }
            return crate::kivio_settings::page_frame(
                "使用统计",
                "请求调试",
                body.into_any_element(),
                cx,
            );
        }
        body = body.child(self.render_activity(cx));
        let mut ranges = div().flex().flex_wrap().gap_2();
        for (range, label) in [
            ("today", "当天"),
            ("1d", "近 1 日"),
            ("7d", "近 7 日"),
            ("30d", "近 30 日"),
            ("90d", "近 90 日"),
            ("365d", "近一年"),
            ("all", "全部"),
        ] {
            ranges = ranges.child(
                Button::new(range)
                    .small()
                    .label(crate::i18n::display(label))
                    .selected(self.range == range)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.range = range;
                        this.offset = 0;
                        this.refresh(cx);
                    })),
            );
        }
        body = body.child(ranges);
        let mut filters = div().flex().flex_wrap().gap_2();
        for (key, selected, options) in [
            (
                "source",
                self.source,
                vec![
                    ("all", "全部来源"),
                    ("chat", "对话"),
                    ("translator", "翻译"),
                    ("screenshot_translation", "截图翻译"),
                    ("lens", "Lens"),
                    ("chat_title_summary", "标题摘要"),
                    ("chat_compression", "上下文压缩"),
                    ("chat_aux_vision", "视觉辅助"),
                    ("chat_aux_video", "视频辅助"),
                    ("chat_image_generation", "图像生成"),
                ],
            ),
            (
                "status",
                self.status,
                vec![
                    ("all", "全部状态"),
                    ("success", "成功"),
                    ("error", "失败"),
                    ("cancelled", "取消"),
                    ("missing_usage", "无用量数据"),
                ],
            ),
        ] {
            let label = options
                .iter()
                .find(|(id, _)| *id == selected)
                .map(|(_, label)| *label)
                .unwrap_or("全部");
            let owner = cx.weak_entity();
            filters = filters.child(
                Button::new(key)
                    .small()
                    .label(crate::i18n::display(label))
                    .dropdown_caret(true)
                    .dropdown_menu(move |mut menu, _, _| {
                        for (id, label) in &options {
                            let owner = owner.clone();
                            let id = *id;
                            menu =
                                menu.item(PopupMenuItem::new(*label).on_click(move |_, _, cx| {
                                    let _ = owner.update(cx, |this, cx| {
                                        if key == "source" {
                                            this.source = id;
                                        } else {
                                            this.status = id;
                                        }
                                        this.offset = 0;
                                        this.refresh(cx);
                                    });
                                }));
                        }
                        menu
                    }),
            );
        }
        body = body.child(filters).child(
            div()
                .flex()
                .gap_2()
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(Input::new(&self.provider).small()),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(Input::new(&self.model).small()),
                )
                .child(
                    Button::new("apply-usage-filter")
                        .small()
                        .label(crate::i18n::text("筛选"))
                        .disabled(self.busy)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.offset = 0;
                            this.refresh(cx);
                        })),
                ),
        );
        if let Some(error) = &self.error {
            body = body.child(div().text_color(cx.theme().danger).child(error.clone()));
        }
        if let Some(stats) = &self.snapshot {
            if let Some(group) = self.group {
                let groups = if group == "providers" {
                    &stats.provider_stats
                } else {
                    &stats.model_stats
                };
                let mut rows = div().v_flex().gap_3();
                if groups.is_empty() {
                    rows = rows.child(crate::i18n::text("当前筛选范围内没有记录。"));
                }
                for item in groups {
                    let provider = item.provider_id.clone().unwrap_or_default();
                    let model = item.model.clone().unwrap_or_default();
                    let success = if item.request_count == 0 {
                        0.
                    } else {
                        item.success_count as f64 * 100. / item.request_count as f64
                    };
                    rows = rows.child(
                        div()
                            .v_flex()
                            .gap_2()
                            .p_4()
                            .rounded(px(12.))
                            .bg(cx.theme().group_box)
                            .child(
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .items_center()
                                    .justify_between()
                                    .gap_3()
                                    .child(item.label.clone())
                                    .child(
                                        Button::new(SharedString::from(format!(
                                            "usage-group-{group}-{}",
                                            item.id
                                        )))
                                        .small()
                                        .label(crate::i18n::text("查看调用"))
                                        .on_click(
                                            cx.listener(move |this, _, window, cx| {
                                                this.provider.update(cx, |input, cx| {
                                                    input.set_value(provider.clone(), window, cx)
                                                });
                                                this.model.update(cx, |input, cx| {
                                                    input.set_value(model.clone(), window, cx)
                                                });
                                                this.group = None;
                                                this.logs = true;
                                                this.offset = 0;
                                                this.refresh(cx);
                                            }),
                                        ),
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .gap_4()
                                    .text_sm()
                                    .child(crate::ui_format!("{} 次调用 · 成功率 {success:.1}%", "{} requests · {success:.1}% success",
                                        item.request_count
                                    ))
                                    .child(format!("{} tokens", item.total_tokens))
                                    .child(format!("${:.4}", item.cost_usd)),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(crate::ui_format!("输入 {} · 输出 {} · 缓存读取 {} · 缓存写入 {} · 平均 {}", "Input {} · Output {} · Cache read {} · Cache write {} · Average {}",
                                        item.input_tokens,
                                        item.output_tokens,
                                        item.cached_input_tokens,
                                        item.cache_creation_input_tokens,
                                        item.average_duration_ms
                                            .map(|ms| format!("{:.1} s", ms / 1000.))
                                            .unwrap_or_else(|| "—".into())
                                    )),
                            ),
                    );
                }
                body = body.child(rows);
            } else if !self.logs {
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
                        crate::ui_format!("{} 次请求", "{} requests", summary.total_requests),
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
                            .bg(cx.theme().group_box)
                            .rounded(px(12.))
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
                        .bg(cx.theme().group_box)
                        .rounded(px(12.))
                        .p_4()
                        .child(crate::i18n::text("Token 趋势"))
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
                    let id = record.id.clone();
                    let expanded = self.expanded.as_ref() == Some(&id);
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
                            .child(format!("{:.1} s", record.duration_ms as f64 / 1000.))
                            .child(
                                Button::new(SharedString::from(format!("usage-detail-{id}")))
                                    .small()
                                    .label(if expanded { crate::i18n::text("收起") } else { crate::i18n::text("详情") })
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.expanded =
                                            if expanded { None } else { Some(id.clone()) };
                                        cx.notify();
                                    })),
                            ),
                    );
                    if expanded {
                        let record = record.clone();
                        let timestamp = chrono::DateTime::from_timestamp(record.created_at, 0)
                            .map(|date| {
                                date.with_timezone(&chrono::Local)
                                    .format("%Y-%m-%d %H:%M:%S")
                                    .to_string()
                            })
                            .unwrap_or_else(|| "—".into());
                        rows =
                            rows.child(
                                div()
                                    .v_flex()
                                    .gap_2()
                                    .p_4()
                                    .rounded(px(10.))
                                    .bg(cx.theme().group_box)
                                    .text_sm()
                                    .child(format!(
                                        "{timestamp} · {} · HTTP {}",
                                        record.api_format,
                                        record
                                            .status_code
                                            .map(|value| value.to_string())
                                            .unwrap_or_else(|| "—".into())
                                    ))
                                    .child(crate::ui_format!("输入 {} · 输出 {} · 推理 {} · 缓存读取 {} · 缓存写入 {}", "Input {} · Output {} · Reasoning {} · Cache read {} · Cache write {}",
                                        optional(record.input_tokens),
                                        optional(record.output_tokens),
                                        optional(record.reasoning_tokens),
                                        optional(record.cached_input_tokens),
                                        optional(record.cache_creation_input_tokens)
                                    ))
                                    .child(crate::ui_format!("首 token {} · 思考等级 {} · 成本 {}", "First token {} · Reasoning level {} · Cost {}",
                                        record
                                            .first_token_ms
                                            .map(|ms| format!("{ms} ms"))
                                            .unwrap_or_else(|| "—".into()),
                                        record.reasoning_effort.as_deref().unwrap_or("—"),
                                        record
                                            .cost_usd
                                            .map(|value| format!("${value:.6}"))
                                            .unwrap_or_else(|| "未提供".into())
                                    ))
                                    .child(crate::ui_format!("用量来源 {} · 成本来源 {}", "Usage source: {} · Cost source: {}",
                                        record.usage_source, record.cost_source
                                    ))
                                    .when_some(record.error_kind.clone(), |view, error| {
                                        view.child(div().text_color(cx.theme().danger).child(error))
                                    })
                                    .child(
                                        div().flex().child(
                                            Button::new(SharedString::from(format!(
                                                "copy-usage-{}",
                                                record.id
                                            )))
                                            .small()
                                            .label(crate::i18n::text("复制完整记录"))
                                            .on_click(move |_, _, cx| {
                                                cx.write_to_clipboard(ClipboardItem::new_string(
                                                    serde_json::to_string_pretty(&record).unwrap_or_default(),
                                                ))
                                            }),
                                        ),
                                    ),
                            );
                    }
                }
                if stats.logs.is_empty() {
                    rows = rows.child(crate::i18n::text("当前范围内没有调用记录。"));
                }
                body = body.child(rows).child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(crate::ui_format!("共 {} 条调用，本页 {} 条。", "{} requests total; {} on this page.",
                            stats.total_logs,
                            stats.logs.len()
                        )),
                );
            }
        }
        if self.logs {
            let total = self.snapshot.as_ref().map_or(0, |stats| stats.total_logs);
            body = body.child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        Button::new("usage-prev")
                            .small()
                            .label(crate::i18n::text("上一页"))
                            .disabled(self.busy || self.offset == 0)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.offset = this.offset.saturating_sub(100);
                                this.refresh(cx);
                            })),
                    )
                    .child(div().child(crate::ui_format!("第 {} 页", "Page {}", self.offset / 100 + 1)))
                    .child(
                        Button::new("usage-next")
                            .small()
                            .label(crate::i18n::text("下一页"))
                            .disabled(self.busy || self.offset + 100 >= total)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.offset += 100;
                                this.refresh(cx);
                            })),
                    ),
            );
        }
        crate::kivio_settings::page_frame(
            "使用统计",
            "模型调用、Token 用量与成本。",
            body.into_any_element(),
            cx,
        )
    }
}

fn optional(value: Option<u64>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "—".into())
}

#[cfg(test)]
mod activity_tests {
    use super::activity_days;
    use chrono::NaiveDate;
    use kivio_model_runtime::usage_ledger;

    #[test]
    fn annual_activity_keeps_leap_day_and_monday_week_totals_separate() {
        let date = |value| NaiveDate::parse_from_str(value, "%Y-%m-%d").unwrap();
        let point = |date: &str, tokens, requests| usage_ledger::UsageTrendPoint {
            date: date.into(),
            total_tokens: tokens,
            requests,
            ..Default::default()
        };
        let days = activity_days(
            &[
                point("2024-02-28", 100, 1),
                point("2024-02-29", 200, 2),
                point("2024-02-29", 50, 1),
                point("2024-03-04", 400, 4),
                point("2024-03-05", 900, 9), // Future data is outside the calendar.
                point("2022-01-01", 900, 9), // So is data before the rolling year.
            ],
            date("2024-03-04"),
        );
        assert_eq!(days.len(), 365);
        let leap_day = days
            .iter()
            .find(|day| day.date == date("2024-02-29"))
            .unwrap();
        assert_eq!((leap_day.tokens, leap_day.requests), (250, 3));
        assert_eq!(leap_day.week_tokens, 350);
        let sunday = days
            .iter()
            .find(|day| day.date == date("2024-03-03"))
            .unwrap();
        assert_eq!(
            (sunday.tokens, sunday.requests, sunday.week_tokens),
            (0, 0, 350)
        );
        let monday = days.last().unwrap();
        assert_eq!((monday.week_tokens, monday.cumulative_tokens), (400, 750));
    }
}
