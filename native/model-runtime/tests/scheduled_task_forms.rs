#[path = "../../zeron-ui/src/task_center/form.rs"]
mod form;
use kivio_model_runtime::scheduled_tasks::types::ScheduleRule;

#[test]
fn every_editor_rule_round_trips_including_cron_and_interval_anchor() {
    for rule in [
        ScheduleRule::Once { at: 1_900_000_047 },
        ScheduleRule::Daily {
            hour: 9,
            minute: 15,
        },
        ScheduleRule::Weekly {
            weekdays: vec![1, 3, 6],
            hour: 8,
            minute: 2,
        },
        ScheduleRule::Monthly {
            days: vec![1, 17, 31],
            hour: 10,
            minute: 0,
        },
        ScheduleRule::Yearly {
            month: 2,
            day: 29,
            hour: 11,
            minute: 20,
        },
        ScheduleRule::Interval {
            minutes: 17,
            anchor_at: Some(123_456_789),
        },
        ScheduleRule::Cron {
            expr: "15 8 * * 1-5".into(),
        },
    ] {
        let (kind, values) = form::fields(Some(&rule));
        assert_eq!(form::parse(kind, &values, Some(&rule)).unwrap(), rule);
        assert!(!form::summary(&rule).is_empty());
    }
}

#[test]
fn invalid_or_incomplete_fields_are_errors_and_interval_edits_keep_anchor() {
    assert!(form::parse(1, &[], None).is_err());
    let (_, mut values) = form::fields(None);
    values[1] = "bad time".into();
    assert!(form::parse(1, &values, None).is_err());
    values[0] = "not a date".into();
    assert!(form::parse(0, &values, None).is_err());
    values[5] = "42".into();
    assert_eq!(
        form::parse(
            5,
            &values,
            Some(&ScheduleRule::Interval {
                minutes: 5,
                anchor_at: Some(100)
            })
        )
        .unwrap(),
        ScheduleRule::Interval {
            minutes: 42,
            anchor_at: Some(100)
        }
    );
}
