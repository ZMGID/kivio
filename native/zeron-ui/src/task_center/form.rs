//! Native schedule form conversion. Existing rules and anchors round-trip unchanged.
use chrono::{Local, NaiveDateTime, TimeZone};
use kivio_model_runtime::scheduled_tasks::types::{RunStatus, ScheduleRule};
pub const KINDS: [&str; 7] = [
    "一次",
    "每天",
    "每周",
    "每月",
    "每年",
    "每隔 N 分钟",
    "Cron",
];

pub fn fields(rule: Option<&ScheduleRule>) -> (usize, Vec<String>) {
    let mut values = vec![
        (Local::now() + chrono::Duration::hours(1))
            .format("%Y-%m-%d %H:%M")
            .to_string(),
        "09:00".into(),
        "1,2,3,4,5".into(),
        "1".into(),
        "01-01".into(),
        "60".into(),
        "0 9 * * *".into(),
    ];
    let time = |hour, minute| format!("{hour:02}:{minute:02}");
    let list = |v: &[u8]| v.iter().map(u8::to_string).collect::<Vec<_>>().join(",");
    let kind = match rule {
        None => 1,
        Some(ScheduleRule::Once { at }) => {
            values[0] = Local
                .timestamp_opt(*at, 0)
                .single()
                .map(|t| t.format("%Y-%m-%d %H:%M:%S").to_string())
                .unwrap_or_default();
            0
        }
        Some(ScheduleRule::Daily { hour, minute }) => {
            values[1] = time(hour, minute);
            1
        }
        Some(ScheduleRule::Weekly {
            weekdays,
            hour,
            minute,
        }) => {
            values[1] = time(hour, minute);
            values[2] = list(weekdays);
            2
        }
        Some(ScheduleRule::Monthly { days, hour, minute }) => {
            values[1] = time(hour, minute);
            values[3] = list(days);
            3
        }
        Some(ScheduleRule::Yearly {
            month,
            day,
            hour,
            minute,
        }) => {
            values[1] = time(hour, minute);
            values[4] = format!("{month:02}-{day:02}");
            4
        }
        Some(ScheduleRule::Interval { minutes, .. }) => {
            values[5] = minutes.to_string();
            5
        }
        Some(ScheduleRule::Cron { expr }) => {
            values[6] = expr.clone();
            6
        }
    };
    (kind, values)
}
pub fn labels(kind: usize) -> Vec<(usize, &'static str)> {
    match kind {
        0 => vec![(0, "日期和时间（YYYY-MM-DD HH:MM，可含秒）")],
        1 => vec![(1, "时间（HH:MM）")],
        2 => vec![
            (1, "时间（HH:MM）"),
            (2, "星期（逗号分隔；0 周日，1 周一，…，6 周六）"),
        ],
        3 => vec![(1, "时间（HH:MM）"), (3, "日期（1–31，逗号分隔）")],
        4 => vec![(1, "时间（HH:MM）"), (4, "月日（MM-DD）")],
        5 => vec![(5, "间隔分钟数")],
        _ => vec![(6, "Cron（分 时 日 月 星期）")],
    }
}
pub fn parse(
    kind: usize,
    v: &[String],
    original: Option<&ScheduleRule>,
) -> Result<ScheduleRule, String> {
    let number = |v: &str| {
        v.trim()
            .parse::<u8>()
            .map_err(|_| "请输入有效整数".to_owned())
    };
    let pair = |v: &str, delimiter| -> Result<(u8, u8), String> {
        let (a, b) = v.split_once(delimiter).ok_or("时间或日期格式无效")?;
        Ok((number(a)?, number(b)?))
    };
    let list = |v: &str| {
        v.split([',', '，'])
            .map(number)
            .collect::<Result<Vec<_>, _>>()
    };
    let get = |ix: usize| {
        v.get(ix)
            .map(|s| s.trim())
            .ok_or_else(|| "计划字段不完整".to_owned())
    };
    let rule = match kind {
        0 => {
            let text = get(0)?;
            let parsed = NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S")
                .or_else(|_| NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M"))
                .map_err(|_| "日期格式应为 YYYY-MM-DD HH:MM")?;
            // A pre-existing one-shot at an ambiguous local time retains its exact instant.
            let at = if let Some(ScheduleRule::Once { at }) =
                original.filter(|r| fields(Some(r)).1[0] == text)
            {
                *at
            } else {
                Local
                    .from_local_datetime(&parsed)
                    .single()
                    .ok_or("该本地时间不存在或有歧义，请选择其他时间")?
                    .timestamp()
            };
            ScheduleRule::Once { at }
        }
        1 => {
            let (hour, minute) = pair(get(1)?, ':')?;
            ScheduleRule::Daily { hour, minute }
        }
        2 => {
            let (hour, minute) = pair(get(1)?, ':')?;
            ScheduleRule::Weekly {
                weekdays: list(get(2)?)?,
                hour,
                minute,
            }
        }
        3 => {
            let (hour, minute) = pair(get(1)?, ':')?;
            ScheduleRule::Monthly {
                days: list(get(3)?)?,
                hour,
                minute,
            }
        }
        4 => {
            let (hour, minute) = pair(get(1)?, ':')?;
            let (month, day) = pair(get(4)?, '-')?;
            ScheduleRule::Yearly {
                month,
                day,
                hour,
                minute,
            }
        }
        5 => ScheduleRule::Interval {
            minutes: get(5)?.parse().map_err(|_| "请输入有效间隔分钟数")?,
            anchor_at: match original {
                Some(ScheduleRule::Interval { anchor_at, .. }) => *anchor_at,
                _ => None,
            },
        },
        6 => ScheduleRule::Cron {
            expr: get(6)?.to_owned(),
        },
        _ => return Err("计划类型无效".into()),
    };
    Ok(rule)
}
pub fn time_label(at: Option<i64>) -> String {
    at.and_then(|t| Local.timestamp_opt(t, 0).single())
        .map(|t| t.format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_else(|| "—".into())
}
pub fn run_status(status: RunStatus) -> &'static str {
    match status {
        RunStatus::Queued => "排队中",
        RunStatus::Running => "运行中",
        RunStatus::Succeeded => "已完成",
        RunStatus::Failed => "失败",
        RunStatus::Skipped => "已跳过",
        RunStatus::Interrupted => "已中断",
    }
}
pub fn summary(rule: &ScheduleRule) -> String {
    let (kind, fields) = fields(Some(rule));
    labels(kind)
        .iter()
        .map(|(ix, _)| fields[*ix].as_str())
        .fold(KINDS[kind].to_owned(), |s, v| format!("{s} · {v}"))
}
