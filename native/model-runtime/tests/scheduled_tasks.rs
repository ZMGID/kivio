use kivio_model_runtime::scheduled_tasks::{ScheduledTasks, types::*};

fn input(id: Option<String>, prompt: &str) -> ScheduledTaskInput {
    ScheduledTaskInput {
        id,
        name: "Daily check".into(),
        prompt: prompt.into(),
        schedule: ScheduleRule::Interval {
            minutes: 5,
            anchor_at: Some(600),
        },
        target: TaskTarget::Conversation {
            conversation_id: "conv_target".into(),
        },
        enabled: Some(true),
    }
}

#[test]
fn native_profile_shares_one_owner_without_reconciling_an_active_run_twice() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("scheduled_tasks");
    let first = kivio_model_runtime::scheduled_tasks::shared(path.clone()).unwrap();
    let task = first
        .save(input(None, "live"), TaskSource::User, 100)
        .unwrap();
    first
        .queue_run(&task, RunTrigger::Manual, None, 101)
        .unwrap();
    let second = kivio_model_runtime::scheduled_tasks::shared(path).unwrap();
    assert!(std::sync::Arc::ptr_eq(&first, &second));
    assert_eq!(second.snapshot().live_runs[0].status, RunStatus::Queued);
}

#[test]
fn save_reopen_preview_validation_and_unknown_fields() {
    let root = tempfile::tempdir().unwrap();
    let owner = ScheduledTasks::load(root.path().into()).unwrap();
    let task = owner
        .save(input(None, "Inspect changes"), TaskSource::User, 100)
        .unwrap();
    assert_eq!(task.next_run_at, Some(600));
    assert!(owner.save(input(None, " "), TaskSource::User, 100).is_err());
    let path = root.path().join("tasks.json");
    let mut saved: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    saved[0]["futureField"] = serde_json::json!({"version": 42});
    saved[0]["schedule"]["futureSchedule"] = true.into();
    std::fs::write(&path, serde_json::to_vec(&saved).unwrap()).unwrap();
    owner.set_enabled(&task.id, false, 101).unwrap();
    let saved: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(saved[0]["futureField"]["version"], 42);
    assert_eq!(saved[0]["schedule"]["futureSchedule"], true);
    let reopened = ScheduledTasks::load(root.path().into()).unwrap();
    assert!(!reopened.get(&task.id).unwrap().enabled);
    assert_eq!(reopened.get(&task.id).unwrap().prompt, "Inspect changes");
    assert!(
        kivio_model_runtime::scheduled_tasks::preview(&ScheduleRule::Cron {
            expr: "bad cron".into()
        })
        .is_err()
    );
    assert_eq!(
        kivio_model_runtime::scheduled_tasks::preview(&ScheduleRule::Cron {
            expr: "0 9 * * 1-5".into()
        })
        .unwrap()
        .len(),
        5
    );
}

#[test]
fn edit_cancels_queued_definition_but_does_not_cancel_already_admitted_reply() {
    let root = tempfile::tempdir().unwrap();
    let owner = ScheduledTasks::load(root.path().into()).unwrap();
    let changed = owner.subscribe();
    let task = owner
        .save(input(None, "old"), TaskSource::User, 100)
        .unwrap();
    assert!(changed.has_changed().unwrap());
    let (queued, receiver) = owner
        .queue_run(&task, RunTrigger::Manual, None, 101)
        .unwrap();
    let receiver = receiver.unwrap();
    let updated = owner
        .save(
            input(Some(task.id.clone()), "replacement"),
            TaskSource::User,
            102,
        )
        .unwrap();
    assert!(*receiver.borrow());
    assert!(!owner.begin_send(&task, &queued.id));
    assert_eq!(owner.runs(&task.id).unwrap()[0].status, RunStatus::Skipped);
    let (run, _) = owner
        .queue_run(&updated, RunTrigger::Manual, None, 103)
        .unwrap();
    assert!(owner.begin_send(&updated, &run.id));
    owner.mark_running(&run.id, 104);
    owner.set_enabled(&task.id, false, 105).unwrap();
    assert_eq!(owner.snapshot().live_runs[0].status, RunStatus::Running);
    owner.finish_run(
        &updated,
        &run.id,
        RunStatus::Failed,
        Some("old failure".into()),
        true,
        106,
    );
    assert_eq!(owner.get(&task.id).unwrap().last_error, None);
    assert!(!owner.get(&task.id).unwrap().enabled);
    assert!(owner.snapshot().live_runs.is_empty());
}

#[test]
fn deleted_run_cannot_resurrect_history_and_missing_target_disables_only_current_task() {
    let root = tempfile::tempdir().unwrap();
    let owner = ScheduledTasks::load(root.path().into()).unwrap();
    let task = owner
        .save(input(None, "prompt"), TaskSource::User, 100)
        .unwrap();
    let (run, _) = owner
        .queue_run(&task, RunTrigger::Manual, None, 101)
        .unwrap();
    assert!(owner.begin_send(&task, &run.id));
    owner.delete(&task.id).unwrap();
    assert!(
        owner
            .finish_run(&task, &run.id, RunStatus::Succeeded, None, false, 102)
            .is_none()
    );
    assert!(owner.runs(&task.id).unwrap().is_empty());
    let task = owner
        .save(input(None, "prompt"), TaskSource::User, 103)
        .unwrap();
    let (run, _) = owner
        .queue_run(&task, RunTrigger::Manual, None, 104)
        .unwrap();
    owner.finish_run(
        &task,
        &run.id,
        RunStatus::Failed,
        Some("target deleted".into()),
        true,
        105,
    );
    assert!(!owner.get(&task.id).unwrap().enabled);
    let reopened = ScheduledTasks::load(root.path().into()).unwrap();
    assert_eq!(
        reopened.runs(&task.id).unwrap()[0].status,
        RunStatus::Failed
    );
}

#[test]
fn due_claims_preserve_anchor_skip_misfires_and_reconcile_inflight_runs() {
    let root = tempfile::tempdir().unwrap();
    let owner = ScheduledTasks::load(root.path().into()).unwrap();
    let task = owner
        .save(input(None, "prompt"), TaskSource::User, 100)
        .unwrap();
    let (fires, missed) = owner.claim_due(600);
    assert_eq!(fires.len(), 1);
    assert!(missed.is_empty());
    assert_eq!(owner.get(&task.id).unwrap().next_run_at, Some(900));
    let (task, at) = fires.into_iter().next().unwrap().into_parts();
    let (run, _) = owner
        .queue_run(&task, RunTrigger::Schedule, Some(at), 600)
        .unwrap();
    let (duplicate, _) = owner
        .queue_run(&task, RunTrigger::Manual, None, 601)
        .unwrap();
    assert_eq!(duplicate.status, RunStatus::Skipped);
    assert!(owner.delete_run(&task.id, &run.id).is_err());
    drop(owner);
    let owner = ScheduledTasks::load(root.path().into()).unwrap();
    assert!(owner.snapshot().live_runs.is_empty());
    assert!(
        owner
            .runs(&task.id)
            .unwrap()
            .iter()
            .any(|r| r.id == run.id && r.status == RunStatus::Interrupted)
    );
    let (fires, missed) = owner.claim_due(1301);
    assert!(fires.is_empty());
    assert_eq!(missed.len(), 1);
    assert_eq!(owner.get(&task.id).unwrap().next_run_at, Some(1500));
}
