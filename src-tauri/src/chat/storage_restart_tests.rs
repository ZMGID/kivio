use super::*;
use std::cell::Cell;
use std::rc::Rc;
use std::time::Instant;

struct CountedReader<R> {
    inner: R,
    bytes: Rc<Cell<usize>>,
}

impl<R: Read> Read for CountedReader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let read = self.inner.read(buffer)?;
        self.bytes.set(self.bytes.get() + read);
        Ok(read)
    }
}

fn conversation(id: &str, content: &str) -> Conversation {
    serde_json::from_value(serde_json::json!({
        "id": id,
        "title": "restart recovery fixture",
        "provider_id": "provider",
        "model": "model",
        "messages": [{"id":"message", "role":"assistant", "content":content, "timestamp":1}],
        "created_at": 1,
        "updated_at": 1
    }))
    .unwrap()
}

fn add_goal(conversation: &mut Conversation, status: super::super::GoalStatus) {
    super::super::goal::start(conversation, "recover interrupted work").unwrap();
    let goal = conversation.goal_state.as_mut().unwrap();
    goal.status = status;
    goal.active_run_id = Some("old-process-run".into());
}

fn read_counted(bytes: &[u8]) -> (Option<RestartGoal>, usize) {
    let count = Rc::new(Cell::new(0));
    let result = read_restart_goal(CountedReader {
        inner: bytes,
        bytes: count.clone(),
    })
    .unwrap();
    (result, count.get())
}

#[test]
fn restart_goal_header_skips_large_ordinary_and_goal_histories() {
    let mut current = conversation("conv_header", &"history ".repeat(131_072));
    for status in [None, Some(super::super::GoalStatus::Active)] {
        if let Some(status) = status {
            add_goal(&mut current, status);
        }
        let encoded = serde_json::to_vec(&current).unwrap();
        let (goal, bytes_read) = read_counted(&encoded);
        assert_eq!(goal.is_some(), status.is_some());
        assert!(encoded.len() > 1_000_000);
        assert!(
            bytes_read <= 8192,
            "Goal recovery read {bytes_read} bytes of a {}-byte conversation",
            encoded.len()
        );
        // Adding the early field/null must remain a normal, compatible Conversation.
        let restored: Conversation = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(restored.messages[0].content, current.messages[0].content);
        assert_eq!(restored.goal_state, current.goal_state);
    }
}

#[test]
fn restart_goal_reads_legacy_field_order_and_missing_goal_without_history_objects() {
    let payload = serde_json::to_string(&"正文 \" } \\ \n ".repeat(10_000)).unwrap();
    let legacy_without_goal =
        format!("{{\"messages\":[{{\"content\":{payload}}}],\"id\":\"conv_old\"}}");
    let (goal, read) = read_counted(legacy_without_goal.as_bytes());
    assert_eq!(goal, None);
    assert_eq!(read, legacy_without_goal.len());

    let legacy_with_goal = format!(
        "{{\"messages\":[{{\"content\":{payload}}}],\"goal_state\":{{\"id\":\"goal_old\",\"version\":7,\"status\":\"verifying\"}}}}"
    );
    let (goal, _) = read_counted(legacy_with_goal.as_bytes());
    assert_eq!(goal.unwrap().version, 7);
}

#[test]
fn restart_goal_candidates_use_files_even_when_index_is_stale_or_invalid() {
    let dir = tempfile::tempdir().unwrap();
    for (id, status) in [
        ("conv_active", Some(super::super::GoalStatus::Active)),
        ("conv_verifying", Some(super::super::GoalStatus::Verifying)),
        ("conv_paused", Some(super::super::GoalStatus::Paused)),
        ("conv_waiting", Some(super::super::GoalStatus::Waiting)),
        ("conv_completed", Some(super::super::GoalStatus::Completed)),
        ("conv_blocked", Some(super::super::GoalStatus::Blocked)),
        ("conv_cancelled", Some(super::super::GoalStatus::Cancelled)),
        ("conv_ordinary", None),
    ] {
        let mut current = conversation(id, "preserve history");
        if let Some(status) = status {
            add_goal(&mut current, status);
        }
        fs::write(
            dir.path().join(format!("{id}.json")),
            serde_json::to_vec(&current).unwrap(),
        )
        .unwrap();
    }
    fs::write(dir.path().join("conv_corrupt.json"), "{broken").unwrap();
    // Neither an incomplete index nor a corrupt index can hide a running Goal.
    for index in [r#"{"conversations":[]}"#, "invalid index"] {
        fs::write(dir.path().join("index.json"), index).unwrap();
        let mut ids: Vec<_> = restart_goal_candidates_in_dir(dir.path())
            .unwrap()
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        ids.sort();
        assert_eq!(ids, ["conv_active", "conv_verifying"]);
    }
}

#[test]
fn restart_goal_pause_rechecks_identity_and_preserves_history() {
    let mut current = conversation("conv_recheck", "must survive restart");
    add_goal(&mut current, super::super::GoalStatus::Verifying);
    let (candidate, _) = read_counted(&serde_json::to_vec(&current).unwrap());
    let candidate = candidate.unwrap();
    let original = current.clone();
    current.goal_state.as_mut().unwrap().version += 1;
    assert!(!super::super::goal::pause_interrupted_goal(&mut current, &candidate).unwrap());
    current = original;
    assert!(super::super::goal::pause_interrupted_goal(&mut current, &candidate).unwrap());
    let goal = current.goal_state.as_ref().unwrap();
    assert_eq!(goal.status, super::super::GoalStatus::Paused);
    assert_eq!(goal.version, candidate.version + 1);
    assert_eq!(goal.active_run_id, None);
    assert_eq!(current.messages[0].content, "must survive restart");
    assert!(!super::super::goal::pause_interrupted_goal(&mut current, &candidate).unwrap());
}

/// Differential fixture using the same full-file loader as the old bulk path.
/// Time is diagnostic only; the read/body-retention assertions are deterministic.
#[test]
fn restart_goal_recovery_resource_regression() {
    let dir = tempfile::tempdir().unwrap();
    let content = "x".repeat(256 * 1024);
    let mut ids = Vec::new();
    let mut baseline_file_bytes = 0;
    for i in 0..24 {
        let id = format!("conv_resource_{i}");
        let current = conversation(&id, &content);
        let encoded = serde_json::to_vec(&current).unwrap();
        baseline_file_bytes += encoded.len();
        fs::write(dir.path().join(format!("{id}.json")), encoded).unwrap();
        ids.push(id);
    }
    let started = Instant::now();
    let old_loaded: Vec<_> = ids
        .iter()
        .map(|id| read_conversation_file(&dir.path().join(format!("{id}.json")), id).unwrap())
        .collect();
    let baseline_elapsed = started.elapsed();
    let retained_body_bytes: usize = old_loaded.iter().map(|c| c.messages[0].content.len()).sum();
    assert_eq!(retained_body_bytes, 24 * 256 * 1024);
    drop(old_loaded);

    let bytes = Rc::new(Cell::new(0));
    let started = Instant::now();
    for id in &ids {
        assert!(read_restart_goal(CountedReader {
            inner: fs::File::open(dir.path().join(format!("{id}.json"))).unwrap(),
            bytes: bytes.clone(),
        })
        .unwrap()
        .is_none());
    }
    let optimized_elapsed = started.elapsed();
    assert!(restart_goal_candidates_in_dir(dir.path())
        .unwrap()
        .is_empty());
    assert!(bytes.get() < baseline_file_bytes / 10);
    eprintln!(
        "restart fixture: 24 ordinary conversations; old full-file reads={baseline_file_bytes} bytes, retained message content={retained_body_bytes} bytes, elapsed={baseline_elapsed:?}; new header reads={} bytes, retained message content=0 bytes, elapsed={optimized_elapsed:?}",
        bytes.get()
    );
}
