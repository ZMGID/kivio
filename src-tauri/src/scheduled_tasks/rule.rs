//! Next-fire computation. Pure functions over unix seconds; calendar rules use
//! the machine's local time zone.

use std::str::FromStr;

use chrono::{DateTime, Datelike, Duration, Local, LocalResult, NaiveTime, TimeZone};
use croner::Cron;

use super::types::ScheduleRule;

pub const MAX_INTERVAL_MINUTES: u32 = 525_600;

pub fn validate(rule: &ScheduleRule) -> Result<(), String> {
    match rule {
        ScheduleRule::Once { .. } => Ok(()),
        ScheduleRule::Interval { minutes, .. } => {
            if (1..=MAX_INTERVAL_MINUTES).contains(minutes) {
                Ok(())
            } else {
                Err(format!("间隔须为 1 到 {MAX_INTERVAL_MINUTES} 分钟"))
            }
        }
        ScheduleRule::Daily { hour, minute } => validate_time(*hour, *minute),
        ScheduleRule::Weekly {
            weekdays,
            hour,
            minute,
        } => {
            if weekdays.is_empty() {
                return Err("每周至少选择一天".into());
            }
            if weekdays.iter().any(|day| *day > 6) {
                return Err("星期取值须为 0（周日）到 6（周六）".into());
            }
            validate_time(*hour, *minute)
        }
        ScheduleRule::Monthly { days, hour, minute } => {
            if days.is_empty() {
                return Err("每月至少选择一天".into());
            }
            if days.iter().any(|day| !(1..=31).contains(day)) {
                return Err("日期须在 1 到 31 之间".into());
            }
            validate_time(*hour, *minute)
        }
        ScheduleRule::Yearly {
            month,
            day,
            hour,
            minute,
        } => {
            if chrono::NaiveDate::from_ymd_opt(2024, u32::from(*month), u32::from(*day)).is_none() {
                return Err("日期无效".into());
            }
            validate_time(*hour, *minute)
        }
        ScheduleRule::Cron { expr } => parse_cron(expr).map(|_| ()),
    }
}

/// First fire time strictly after `after`, or None when the rule is exhausted.
pub fn next_after(rule: &ScheduleRule, after: i64) -> Result<Option<i64>, String> {
    validate(rule)?;
    Ok(match rule {
        ScheduleRule::Once { at } => (*at > after).then_some(*at),
        ScheduleRule::Interval { minutes, anchor_at } => {
            let step = i64::from(*minutes) * 60;
            let anchor = anchor_at.unwrap_or(after + step);
            if anchor > after {
                Some(anchor)
            } else {
                Some(anchor + ((after - anchor) / step + 1) * step)
            }
        }
        ScheduleRule::Daily { hour, minute } => next_calendar(after, *hour, *minute, 8, |_| true),
        ScheduleRule::Weekly {
            weekdays,
            hour,
            minute,
        } => next_calendar(after, *hour, *minute, 8, |date| {
            let day = date.weekday().num_days_from_sunday() as u8;
            weekdays.contains(&day)
        }),
        // Jan 31 → Mar 31 is the longest gap between two months that have a given day.
        ScheduleRule::Monthly { days, hour, minute } => {
            next_calendar(after, *hour, *minute, 64, |date| {
                days.contains(&(date.day() as u8))
            })
        }
        // Feb 29 can be eight years away across a non-leap century.
        ScheduleRule::Yearly {
            month,
            day,
            hour,
            minute,
        } => next_calendar(after, *hour, *minute, 366 * 8 + 2, |date| {
            date.month() == u32::from(*month) && date.day() == u32::from(*day)
        }),
        ScheduleRule::Cron { expr } => {
            let cron = parse_cron(expr)?;
            let start = local(after)?;
            cron.find_next_occurrence(&start, false)
                .ok()
                .map(|next| next.timestamp())
        }
    })
}

/// Up to `count` upcoming fire times after `after`.
pub fn preview(rule: &ScheduleRule, after: i64, count: usize) -> Result<Vec<i64>, String> {
    let mut times = Vec::with_capacity(count);
    let mut cursor = after;
    while times.len() < count {
        let Some(next) = next_after(rule, cursor)? else {
            break;
        };
        times.push(next);
        cursor = next;
    }
    Ok(times)
}

fn validate_time(hour: u8, minute: u8) -> Result<(), String> {
    if hour > 23 || minute > 59 {
        return Err("时间须在 00:00 到 23:59 之间".into());
    }
    Ok(())
}

fn parse_cron(expr: &str) -> Result<Cron, String> {
    let expr = expr.trim();
    if expr.split_whitespace().count() != 5 {
        return Err("Cron 表达式须为 5 段：分 时 日 月 周".into());
    }
    Cron::from_str(expr).map_err(|err| format!("Cron 表达式无效：{err}"))
}

fn local(timestamp: i64) -> Result<DateTime<Local>, String> {
    Local
        .timestamp_opt(timestamp, 0)
        .single()
        .ok_or_else(|| "时间超出范围".to_string())
}

fn next_calendar(
    after: i64,
    hour: u8,
    minute: u8,
    horizon_days: i64,
    day_matches: impl Fn(chrono::NaiveDate) -> bool,
) -> Option<i64> {
    let start = local(after).ok()?.date_naive();
    let time = NaiveTime::from_hms_opt(u32::from(hour), u32::from(minute), 0)?;
    // The horizon covers the rule's longest gap plus one day for a DST gap.
    (0..horizon_days).find_map(|offset| {
        let date = start + Duration::days(offset);
        if !day_matches(date) {
            return None;
        }
        let candidate = match Local.from_local_datetime(&date.and_time(time)) {
            LocalResult::Single(value) => value,
            LocalResult::Ambiguous(first, _) => first,
            // Skipped by a DST jump: this day has no such wall-clock time.
            LocalResult::None => return None,
        };
        let timestamp = candidate.timestamp();
        (timestamp > after).then_some(timestamp)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> i64 {
        Local
            .with_ymd_and_hms(y, mo, d, h, mi, 0)
            .single()
            .unwrap()
            .timestamp()
    }

    #[test]
    fn once_fires_only_while_in_the_future() {
        let rule = ScheduleRule::Once { at: 1_000 };
        assert_eq!(next_after(&rule, 999).unwrap(), Some(1_000));
        assert_eq!(next_after(&rule, 1_000).unwrap(), None);
    }

    #[test]
    fn interval_stays_on_anchor_grid_after_late_wakeups() {
        let rule = ScheduleRule::Interval {
            minutes: 30,
            anchor_at: Some(10_000),
        };
        assert_eq!(next_after(&rule, 5_000).unwrap(), Some(10_000));
        assert_eq!(next_after(&rule, 10_000).unwrap(), Some(11_800));
        // Woke up 50 minutes late: next slot is on the grid, not now + 30 min.
        assert_eq!(next_after(&rule, 13_000).unwrap(), Some(13_600));
    }

    #[test]
    fn daily_rolls_to_tomorrow_once_today_passed() {
        let rule = ScheduleRule::Daily { hour: 9, minute: 0 };
        assert_eq!(
            next_after(&rule, at(2026, 3, 10, 8, 59)).unwrap(),
            Some(at(2026, 3, 10, 9, 0))
        );
        assert_eq!(
            next_after(&rule, at(2026, 3, 10, 9, 0)).unwrap(),
            Some(at(2026, 3, 11, 9, 0))
        );
    }

    #[test]
    fn weekly_picks_next_selected_weekday() {
        // 2026-03-13 is a Friday; next Monday/Wednesday slot is 2026-03-16.
        let rule = ScheduleRule::Weekly {
            weekdays: vec![1, 3],
            hour: 8,
            minute: 30,
        };
        assert_eq!(
            next_after(&rule, at(2026, 3, 13, 12, 0)).unwrap(),
            Some(at(2026, 3, 16, 8, 30))
        );
    }

    #[test]
    fn cron_uses_local_time_and_is_strictly_after() {
        let rule = ScheduleRule::Cron {
            expr: "0 9 * * 1-5".into(),
        };
        // Saturday 09:00 → Monday 09:00.
        assert_eq!(
            next_after(&rule, at(2026, 3, 14, 9, 0)).unwrap(),
            Some(at(2026, 3, 16, 9, 0))
        );
    }

    #[test]
    fn invalid_rules_are_rejected_with_reason() {
        assert!(validate(&ScheduleRule::Cron {
            expr: "* * *".into()
        })
        .is_err());
        assert!(validate(&ScheduleRule::Cron {
            expr: "0 0 9 * * *".into()
        })
        .is_err());
        assert!(validate(&ScheduleRule::Cron {
            expr: "61 * * * *".into()
        })
        .is_err());
        assert!(validate(&ScheduleRule::Weekly {
            weekdays: vec![],
            hour: 9,
            minute: 0
        })
        .is_err());
        assert!(validate(&ScheduleRule::Daily {
            hour: 24,
            minute: 0
        })
        .is_err());
        assert!(validate(&ScheduleRule::Interval {
            minutes: 0,
            anchor_at: None
        })
        .is_err());
        assert!(validate(&ScheduleRule::Monthly {
            days: vec![32],
            hour: 9,
            minute: 0
        })
        .is_err());
        assert!(validate(&ScheduleRule::Yearly {
            month: 2,
            day: 30,
            hour: 9,
            minute: 0
        })
        .is_err());
    }

    #[test]
    fn monthly_skips_months_without_the_day() {
        let rule = ScheduleRule::Monthly {
            days: vec![31],
            hour: 9,
            minute: 0,
        };
        // After Jan 31 the next 31st is March 31 (February and April have none).
        assert_eq!(
            next_after(&rule, at(2026, 1, 31, 10, 0)).unwrap(),
            Some(at(2026, 3, 31, 9, 0))
        );
    }

    #[test]
    fn yearly_leap_day_waits_for_a_leap_year() {
        let rule = ScheduleRule::Yearly {
            month: 2,
            day: 29,
            hour: 8,
            minute: 0,
        };
        assert_eq!(
            next_after(&rule, at(2026, 3, 1, 0, 0)).unwrap(),
            Some(at(2028, 2, 29, 8, 0))
        );
    }

    #[test]
    fn preview_stops_when_rule_is_exhausted() {
        let rule = ScheduleRule::Once { at: 2_000 };
        assert_eq!(preview(&rule, 0, 5).unwrap(), vec![2_000]);
    }
}
