//! Owner-scoped OMP `wait` for subagent jobs.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::future::Future;
use std::sync::{LazyLock, Mutex};
use std::time::Duration;
use tokio::time::Instant;

use serde_json::{json, Value};

use crate::mcp::native_registry::NativeToolFuture;
use crate::mcp::types::{ChatToolDefinition, McpToolCallResult};

use super::jobs::{JobManager, JobSnapshot, JobStatus};
use super::{prompts, SubAgentCallCtx, WAIT_TOOL_NAME};

const LADDER: [Duration; 5] = [
    Duration::from_secs(5),
    Duration::from_secs(10),
    Duration::from_secs(30),
    Duration::from_secs(60),
    Duration::from_secs(300),
];
const LADDER_RESET: Duration = Duration::from_secs(60);
const SETTLE_WINDOW: Duration = Duration::from_millis(1500);
const NOTHING_TO_WAIT_FOR: &str = "Nothing to wait for: no background subagent job you started is running.";

pub fn definition() -> ChatToolDefinition {
    ChatToolDefinition {
        id: "native__wait".into(),
        name: WAIT_TOOL_NAME.into(),
        description: prompts::WAIT_TOOL.into(),
        source: "native".into(),
        server_id: None,
        server_name: Some("Kivio".into()),
        input_schema: json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        }),
        sensitive: false,
        annotations: Some(json!({
            "readOnlyHint": true,
            "destructiveHint": false,
            "openWorldHint": false
        })),
        output_schema: None,
    }
}

#[derive(Clone, Copy)]
struct IdleStreak {
    rung: usize,
    ended_at: Instant,
    waited: Duration,
}

#[derive(Default)]
struct WaitLadders {
    owners: HashMap<String, HashMap<String, IdleStreak>>,
}

impl WaitLadders {
    fn begin(&mut self, root: &str, owner: &str, now: Instant) -> (usize, Duration) {
        self.owners.retain(|_, owners| {
            owners.retain(|_, idle| now.saturating_duration_since(idle.ended_at) < LADDER_RESET);
            !owners.is_empty()
        });
        match self.owners.get_mut(root).and_then(|owners| owners.remove(owner)) {
            Some(idle) => ((idle.rung + 1).min(LADDER.len() - 1), idle.waited),
            None => (0, Duration::ZERO),
        }
    }

    fn elapsed(&mut self, root: &str, owner: &str, rung: usize, waited: Duration, now: Instant) {
        self.owners.entry(root.into()).or_default().insert(
            owner.into(),
            IdleStreak { rung, ended_at: now, waited: waited + LADDER[rung] },
        );
    }
}

static WAIT_LADDERS: LazyLock<Mutex<WaitLadders>> = LazyLock::new(|| Mutex::new(WaitLadders::default()));

pub fn dispatch(ctx: SubAgentCallCtx<'_>) -> NativeToolFuture<'_> {
    Box::pin(async move {
        if !ctx.arguments.as_object().is_some_and(|args| args.is_empty()) {
            return Err("wait accepts no arguments".into());
        }
        let root = ctx.native_ctx.conversation_id.as_str();
        let owner = ctx.state.sub_agents.owner_for_run(&ctx.native_ctx.run_id);
        let child_conversation = ctx.state.sub_agents.registry.get(root, &owner)
            .map(|agent| agent.child_conversation_id);
        let generation_conversation = child_conversation.as_deref().unwrap_or(root);
        let cancelled = ctx.state.chat_runtime().wait_for_generation_inactive(
            generation_conversation,
            ctx.native_ctx.generation,
        );
        let steering = ctx.state.chat_runtime().wait_for_steering(generation_conversation);
        let _child_progress = (owner != super::MAIN_AGENT_ID).then(|| ChildWaitProgress {
            jobs: &ctx.state.sub_agents.jobs,
            root,
            owner: &owner,
        });
        wait_with_updates(
            &ctx.state.sub_agents.jobs,
            &WAIT_LADDERS,
            root,
            &owner,
            cancelled,
            steering,
            |snapshots| {
                if owner != super::MAIN_AGENT_ID {
                    ctx.state.sub_agents.jobs.report_wait_progress(root, &owner, Some(json!({
                        "op": "wait", "jobs": snapshots,
                    })));
                }
                let Some(id) = ctx.native_ctx.tool_call_id.as_deref() else { return };
                let Some(mut tool) = ctx.state.chat_protocol()
                    .running_snapshot(root, &ctx.native_ctx.run_id, &ctx.native_ctx.message_id)
                    .and_then(|snapshot| snapshot.tools.into_iter()
                        .find(|tool| tool.id == id && tool.round == ctx.native_ctx.round))
                else { return };
                tool.structured_content = Some(json!({ "op": "wait", "jobs": snapshots }));
                crate::chat::protocol::emit_run_event(
                    ctx.app,
                    &ctx.native_ctx.run_id,
                    crate::chat::protocol::ChatRunEvent::ToolUpdated { tool },
                );
            },
        ).await
    })
}

struct ChildWaitProgress<'a> {
    jobs: &'a JobManager,
    root: &'a str,
    owner: &'a str,
}

impl Drop for ChildWaitProgress<'_> {
    fn drop(&mut self) {
        self.jobs.report_wait_progress(self.root, self.owner, None);
    }
}

struct WatchedJobs<'a> {
    jobs: &'a JobManager,
    ids: Vec<String>,
}

impl Drop for WatchedJobs<'_> {
    fn drop(&mut self) {
        self.jobs.unwatch_jobs(&self.ids);
    }
}

#[cfg(test)]
async fn wait(
    jobs: &JobManager,
    ladders: &Mutex<WaitLadders>,
    root: &str,
    owner: &str,
    cancelled: impl Future<Output = ()>,
) -> Result<McpToolCallResult, String> {
    wait_with_updates(jobs, ladders, root, owner, cancelled, std::future::pending(), |_| {}).await
}

async fn wait_with_updates(
    jobs: &JobManager,
    ladders: &Mutex<WaitLadders>,
    root: &str,
    owner: &str,
    cancelled: impl Future<Output = ()>,
    steering: impl Future<Output = ()>,
    on_progress: impl Fn(&[JobSnapshot]) + Send + Sync,
) -> Result<McpToolCallResult, String> {
    tokio::select! {
        biased;
        _ = cancelled => Err("cancelled".into()),
        _ = steering => {
            let mut output = result("Wait interrupted by user steering.".into(), &[]);
            output.raw["useless"] = json!(false);
            output.raw["interrupted"] = json!(true);
            output.structured_content = Some(output.raw.clone());
            Ok(output)
        },
        result = wait_for_jobs(jobs, ladders, root, owner, &on_progress) => Ok(result),
    }
}

async fn wait_for_jobs(
    jobs: &JobManager,
    ladders: &Mutex<WaitLadders>,
    root: &str,
    owner: &str,
    on_progress: &(impl Fn(&[JobSnapshot]) + Sync),
) -> McpToolCallResult {
    let (rung, previously_waited) = ladders.lock().unwrap_or_else(|e| e.into_inner())
        .begin(root, owner, Instant::now());
    // Subscribe before taking the snapshot, so a settlement cannot be missed.
    let mut changed = jobs.subscribe();
    let (claimed, ids) = jobs.begin_wait(root, owner);
    let watched = WatchedJobs { jobs, ids };
    if !claimed.is_empty() {
        return snapshot(jobs, claimed, &watched.ids);
    }
    if watched.ids.is_empty() {
        return result(NOTHING_TO_WAIT_FOR.into(), &[]);
    }

    let emit_progress = || {
        let snapshots: Vec<_> = watched.ids.iter().filter_map(|id| jobs.get(id)).collect();
        on_progress(&snapshots);
    };
    emit_progress();
    let mut progress = tokio::time::interval_at(
        tokio::time::Instant::now() + Duration::from_millis(500),
        Duration::from_millis(500),
    );

    let deadline = tokio::time::Instant::now() + LADDER[rung];
    let mut settle_deadline = None;
    let timed_out = loop {
        let mut any_running = false;
        let mut any_settled = false;
        for id in &watched.ids {
            match jobs.status(id) {
                Some(JobStatus::Running) => any_running = true,
                _ => any_settled = true,
            }
        }
        if any_settled && settle_deadline.is_none() {
            settle_deadline = Some(tokio::time::Instant::now() + SETTLE_WINDOW);
        }
        if !any_running {
            break false;
        }
        let until = settle_deadline.map_or(deadline, |settle| settle.min(deadline));
        tokio::select! {
            _ = tokio::time::sleep_until(until) => break until == deadline,
            _ = progress.tick() => emit_progress(),
            changed = changed.changed() => {
                if changed.is_err() {
                    break false;
                }
            }
        }
    };

    let mut output = snapshot(jobs, Vec::new(), &watched.ids);
    if timed_out && output.structured_content.as_ref().is_some_and(|details| details["useless"] == true) {
        ladders.lock().unwrap_or_else(|e| e.into_inner())
            .elapsed(root, owner, rung, previously_waited, Instant::now());
        if rung > 0 {
            let waited = format_duration((previously_waited + LADDER[rung]).as_millis() as u64);
            write!(output.content, "\n\n<system-notice>\nNothing new after {waited} of back-to-back waits. Waiting is not progress: each response MUST advance the task. Make as much progress as you can on everything that does not need what you are waiting on; call `wait` again only once nothing else is left. Results arrive on their own.\n</system-notice>").unwrap();
        }
    }
    output
}

#[derive(Clone, Copy)]
enum Delivery {
    Recovered,
    Direct,
    AlreadyConsumed,
    None,
}

fn snapshot(jobs: &JobManager, claimed: Vec<JobSnapshot>, watched_ids: &[String]) -> McpToolCallResult {
    let mut rows: Vec<(JobSnapshot, Delivery)> = claimed.into_iter()
        .map(|job| (job, Delivery::Recovered)).collect();
    let mut claimed: HashMap<_, _> = jobs.take_wait_results(watched_ids).into_iter()
        .map(|job| (job.id.clone(), job)).collect();
    for id in watched_ids {
        if let Some(job) = claimed.remove(id) {
            rows.push((job, Delivery::Direct));
        } else if let Some(job) = jobs.get(id) {
            match job.status {
                JobStatus::Running | JobStatus::Cancelled => rows.push((job, Delivery::None)),
                _ if job.consumed => rows.push((job, Delivery::AlreadyConsumed)),
                _ => {
                    // It settled after the batch claim; claim it before unwatching.
                    if let Some(job) = jobs.take_wait_results(std::slice::from_ref(id)).into_iter().next() {
                        rows.push((job, Delivery::Direct));
                    } else if let Some(job) = jobs.get(id).filter(|job| job.consumed) {
                        rows.push((job, Delivery::AlreadyConsumed));
                    }
                }
            }
        }
    }

    let mut text = String::new();
    let completed = rows.iter().filter(|(job, _)| job.status != JobStatus::Running).count();
    if completed > 0 {
        write!(text, "## Completed ({completed})\n").unwrap();
        for (job, delivery) in rows.iter().filter(|(job, _)| job.status != JobStatus::Running) {
            let status = match job.status {
                JobStatus::Completed => "completed",
                JobStatus::Failed => "failed",
                JobStatus::Cancelled => "cancelled",
                JobStatus::Running => unreachable!(),
            };
            write!(text, "\n### {} [task] — {status}\nLabel: {}\n", job.id, job.label).unwrap();
            if job.id != job.agent_id {
                if job.status == JobStatus::Completed {
                    writeln!(text, "Agent: agent://{}, history://{}", job.agent_id, job.agent_id).unwrap();
                } else {
                    writeln!(text, "Agent: history://{}", job.agent_id).unwrap();
                }
            }
            match delivery {
                Delivery::Recovered => writeln!(text, "Delivery: not auto-delivered; recovered by this snapshot.").unwrap(),
                Delivery::Direct => writeln!(text, "Delivery: delivered directly to wait.").unwrap(),
                Delivery::AlreadyConsumed => writeln!(text, "Delivery: already delivered or recovered.").unwrap(),
                Delivery::None => {}
            }
            if let Some(body) = job.result.as_deref().filter(|body| !body.is_empty()) {
                if job.status == JobStatus::Failed {
                    writeln!(text, "Error: {body}").unwrap();
                } else {
                    writeln!(text, "```\n{body}\n```").unwrap();
                }
            }
        }
    }
    let running = rows.len() - completed;
    if running > 0 {
        if !text.is_empty() {
            text.push('\n');
        }
        write!(text, "## Still Running ({running})\n").unwrap();
        for (job, _) in rows.iter().filter(|(job, _)| job.status == JobStatus::Running) {
            write!(text, "\n- `{}` [task] — {}{}", job.id, job.label, progress_suffix(job)).unwrap();
        }
    }
    if text.is_empty() {
        text.push_str("No background jobs.");
    }
    let snapshots: Vec<_> = rows.iter().map(|(job, _)| job).collect();
    result(text.trim_end().to_string(), &snapshots)
}

fn progress_suffix(job: &JobSnapshot) -> String {
    let Some(progress) = job.progress.as_ref() else { return String::new() };
    let mut parts = Vec::new();
    if let Some(requests) = progress.get("requests").and_then(Value::as_u64).filter(|n| *n > 0) {
        parts.push(format!("{requests} req"));
    }
    if let Some(cost) = progress.get("cost").and_then(Value::as_f64).filter(|n| *n > 0.0) {
        parts.push(if cost < 0.005 { "<$0.01".into() } else { format!("${cost:.2}") });
    }
    if let Some(tokens) = progress.get("contextTokens").and_then(Value::as_u64).filter(|n| *n > 0) {
        let mut context = format!("ctx {}", format_number(tokens));
        if let Some(window) = progress.get("contextWindow").and_then(Value::as_u64).filter(|n| *n > 0) {
            write!(context, "/{}", format_number(window)).unwrap();
        }
        parts.push(context);
    }
    if let Some(percent) = progress.get("completionPercent").and_then(Value::as_f64) {
        parts.push(format!("~{percent}%"));
    }
    if let Some(stage) = progress.get("stage").and_then(Value::as_str) {
        let stage = stage.split_whitespace().collect::<Vec<_>>().join(" ");
        if !stage.is_empty() {
            parts.push(format!("stage: {stage}"));
        }
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!(" — {} · {}", format_duration(job.duration_ms().max(0) as u64), parts.join(" · "))
    }
}

fn format_number(value: u64) -> String {
    let (divisor, suffix) = if value >= 1_000_000 { (1_000_000, "M") }
        else if value >= 1_000 { (1_000, "K") }
        else { return value.to_string() };
    if value % divisor == 0 {
        format!("{}{suffix}", value / divisor)
    } else {
        format!("{:.1}{suffix}", value as f64 / divisor as f64)
    }
}

fn format_duration(ms: u64) -> String {
    if ms < 1_000 {
        return format!("{ms}ms");
    }
    let seconds = ms / 1_000;
    if seconds < 60 {
        format!("{seconds}s")
    } else if seconds < 3_600 {
        let remainder = seconds % 60;
        if remainder == 0 { format!("{}m", seconds / 60) } else { format!("{}m{remainder}s", seconds / 60) }
    } else {
        let minutes = (seconds % 3_600) / 60;
        if minutes == 0 { format!("{}h", seconds / 3_600) } else { format!("{}h{minutes}m", seconds / 3_600) }
    }
}

fn result(content: String, jobs: &[&JobSnapshot]) -> McpToolCallResult {
    let details = json!({ "op": "wait", "jobs": jobs, "useless": jobs.iter().all(|job| job.status == JobStatus::Running) });
    McpToolCallResult {
        content,
        is_error: false,
        raw: details.clone(),
        artifacts: Vec::new(),
        structured_content: Some(details),
        follow_up_user_messages: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::pending;
    use std::sync::Arc;

    #[test]
    fn definition_has_no_parameters() {
        let tool = definition();
        assert_eq!(tool.name, "wait");
        assert_eq!(tool.description, prompts::WAIT_TOOL);
        assert_eq!(tool.input_schema, json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        }));
    }

    #[test]
    fn ladder_climbs_caps_resets_and_is_scoped_to_owner_and_root() {
        let mut ladders = WaitLadders::default();
        let now = Instant::now();
        assert_eq!(ladders.begin("root", "Main", now), (0, Duration::ZERO));
        ladders.elapsed("root", "Main", 0, Duration::ZERO, now);
        assert_eq!(ladders.begin("root", "Other", now), (0, Duration::ZERO));
        assert_eq!(ladders.begin("other-root", "Main", now), (0, Duration::ZERO));
        let (rung, waited) = ladders.begin("root", "Main", now + Duration::from_secs(59));
        assert_eq!((rung, waited), (1, Duration::from_secs(5)));
        ladders.elapsed("root", "Main", rung, waited, now);
        for (expected_rung, expected_waited) in [(2, 15), (3, 45), (4, 105), (4, 405)] {
            let (rung, waited) = ladders.begin("root", "Main", now);
            assert_eq!((rung, waited), (expected_rung, Duration::from_secs(expected_waited)));
            ladders.elapsed("root", "Main", rung, waited, now);
        }
        assert_eq!(ladders.begin("root", "Main", now + LADDER_RESET), (0, Duration::ZERO));
    }

    #[tokio::test]
    async fn wait_returns_undelivered_first_and_consumes_once() {
        let jobs = JobManager::default();
        let ladders = Mutex::new(WaitLadders::default());
        let (done, _) = jobs.register("root", "Main", "Done", "completed work");
        let (running, _) = jobs.register("root", "Main", "Running", "ongoing work");
        let (other, _) = jobs.register("root", "Other", "Private", "private work");
        jobs.settle(&done, Ok("unique result".into()));
        jobs.settle(&other, Ok("private result".into()));
        let output = tokio::time::timeout(
            Duration::from_millis(100),
            wait(&jobs, &ladders, "root", "Main", pending()),
        ).await.expect("undelivered result must return without waiting").unwrap();
        assert!(output.content.contains("## Completed (1)"));
        assert!(output.content.contains("unique result"));
        assert!(output.content.contains("## Still Running (1)"));
        assert!(!output.content.contains("private result"));
        assert_eq!(output.structured_content.as_ref().unwrap()["jobs"][0]["result"], "unique result");
        assert_eq!(output.structured_content.as_ref().unwrap()["useless"], false);
        assert!(jobs.take_deliverable("root", "Main").is_empty());
        assert!(jobs.cancel("root", &running, "Main"));
        let second = wait(&jobs, &ladders, "root", "Main", pending()).await.unwrap();
        assert_eq!(second.content, NOTHING_TO_WAIT_FOR);
        assert_eq!(second.structured_content.as_ref().unwrap()["useless"], true);
        assert!(!second.content.contains("unique result"));
        assert_eq!(jobs.take_deliverable("root", "Other").len(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn wait_collects_nearby_settlements_and_stops_when_all_settle() {
        let jobs = Arc::new(JobManager::default());
        let ladders = Arc::new(Mutex::new(WaitLadders::default()));
        let (first, _) = jobs.register("root", "Main", "First", "one");
        let (second, _) = jobs.register("root", "Main", "Second", "two");
        let task_jobs = jobs.clone();
        let task = tokio::spawn(async move {
            wait(&task_jobs, &ladders, "root", "Main", pending()).await.unwrap()
        });
        tokio::task::yield_now().await;
        jobs.settle(&first, Ok("first result".into()));
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_millis(1499)).await;
        assert!(!task.is_finished());
        jobs.settle(&second, Ok("second result".into()));
        let output = task.await.unwrap();
        assert!(output.content.contains("## Completed (2)"));
        assert!(output.content.contains("first result"));
        assert!(output.content.contains("second result"));
        assert!(jobs.take_deliverable("root", "Main").is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn settle_window_returns_completed_and_still_running() {
        let jobs = Arc::new(JobManager::default());
        let ladders = Arc::new(Mutex::new(WaitLadders::default()));
        let (first, _) = jobs.register("root", "Main", "First", "one");
        let (second, _) = jobs.register("root", "Main", "Second", "two");
        let task_jobs = jobs.clone();
        let task = tokio::spawn(async move {
            wait(&task_jobs, &ladders, "root", "Main", pending()).await.unwrap()
        });
        tokio::task::yield_now().await;
        jobs.settle(&first, Ok("first result".into()));
        tokio::task::yield_now().await;
        tokio::time::advance(SETTLE_WINDOW).await;
        let output = task.await.unwrap();
        assert!(output.content.contains("## Completed (1)"));
        assert!(output.content.contains("first result"));
        assert!(output.content.contains("## Still Running (1)"));
        assert!(jobs.take_deliverable("root", "Main").is_empty());
        jobs.settle(&second, Ok("later result".into()));
        assert_eq!(jobs.take_deliverable("root", "Main").len(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn elapsed_waits_use_the_next_ladder_window() {
        let jobs = Arc::new(JobManager::default());
        let ladders = Arc::new(Mutex::new(WaitLadders::default()));
        jobs.register("root", "Main", "Running", "ongoing");
        let task_jobs = jobs.clone();
        let task_ladders = ladders.clone();
        let first = tokio::spawn(async move {
            wait(&task_jobs, &task_ladders, "root", "Main", pending()).await.unwrap()
        });
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(5)).await;
        assert!(first.await.unwrap().content.contains("## Still Running (1)"));

        let task_jobs = jobs.clone();
        let second = tokio::spawn(async move {
            wait(&task_jobs, &ladders, "root", "Main", pending()).await.unwrap()
        });
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(9)).await;
        assert!(!second.is_finished());
        tokio::time::advance(Duration::from_secs(1)).await;
        let output = second.await.unwrap();
        assert!(output.content.contains("## Still Running (1)"));
        assert_eq!(output.structured_content.as_ref().unwrap()["useless"], true);
        assert!(output.content.contains("Nothing new after 15s"));
    }

    #[tokio::test]
    async fn cancellation_during_collection_preserves_completed_result() {
        let jobs = Arc::new(JobManager::default());
        let ladders = Arc::new(Mutex::new(WaitLadders::default()));
        let (first, _) = jobs.register("root", "Main", "First", "one");
        let (second, _) = jobs.register("root", "Main", "Second", "two");
        let (cancel, cancelled) = tokio::sync::oneshot::channel::<()>();
        let task_jobs = jobs.clone();
        let task = tokio::spawn(async move {
            wait(&task_jobs, &ladders, "root", "Main", async { let _ = cancelled.await; }).await
        });
        tokio::task::yield_now().await;
        jobs.settle(&first, Ok("preserved".into()));
        tokio::task::yield_now().await;
        assert!(jobs.take_deliverable("root", "Main").is_empty());
        cancel.send(()).unwrap();
        assert_eq!(task.await.unwrap().unwrap_err(), "cancelled");
        let delivered = jobs.take_deliverable("root", "Main");
        assert_eq!(delivered.len(), 1);
        assert_eq!(delivered[0].result.as_deref(), Some("preserved"));
        assert!(jobs.cancel("root", &second, "Main"));
    }

    #[tokio::test]
    async fn cancellation_and_dropped_future_release_watchers() {
        let jobs = Arc::new(JobManager::default());
        let ladders = Arc::new(Mutex::new(WaitLadders::default()));
        let (first, _) = jobs.register("root", "Main", "First", "one");
        let (cancel, cancelled) = tokio::sync::oneshot::channel::<()>();
        let task_jobs = jobs.clone();
        let task_ladders = ladders.clone();
        let task = tokio::spawn(async move {
            wait(&task_jobs, &task_ladders, "root", "Main", async { let _ = cancelled.await; }).await
        });
        tokio::task::yield_now().await;
        cancel.send(()).unwrap();
        assert_eq!(task.await.unwrap().unwrap_err(), "cancelled");
        jobs.settle(&first, Ok("after cancellation".into()));
        assert_eq!(jobs.take_deliverable("root", "Main").len(), 1);

        let (second, _) = jobs.register("root", "Main", "Second", "two");
        let task_jobs = jobs.clone();
        let task = tokio::spawn(async move {
            wait(&task_jobs, &ladders, "root", "Main", pending()).await
        });
        tokio::task::yield_now().await;
        task.abort();
        let _ = task.await;
        jobs.settle(&second, Ok("after drop".into()));
        assert_eq!(jobs.take_deliverable("root", "Main").len(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn wait_emits_periodic_non_consuming_snapshots_and_cancellation_releases_them() {
        let jobs = Arc::new(JobManager::default());
        let ladders = Arc::new(Mutex::new(WaitLadders::default()));
        let updates = Arc::new(Mutex::new(Vec::<Vec<JobSnapshot>>::new()));
        let (job, _) = jobs.register("root", "Main", "Scout", "inspect");
        let (cancel, cancelled) = tokio::sync::oneshot::channel::<()>();
        let task = {
            let jobs = jobs.clone();
            let updates = updates.clone();
            tokio::spawn(async move {
                wait_with_updates(&jobs, &ladders, "root", "Main", async { let _ = cancelled.await; },
                    pending(), |snapshot| updates.lock().unwrap().push(snapshot.to_vec())).await
            })
        };
        tokio::task::yield_now().await;
        assert_eq!(updates.lock().unwrap().len(), 1);
        jobs.report_progress(&job, json!({ "stage": "reading" }));
        tokio::time::advance(Duration::from_millis(500)).await;
        tokio::task::yield_now().await;
        assert_eq!(updates.lock().unwrap().last().unwrap()[0].progress.as_ref().unwrap()["stage"], "reading");
        cancel.send(()).unwrap();
        assert_eq!(task.await.unwrap().unwrap_err(), "cancelled");
        jobs.settle(&job, Ok("after interrupt".into()));
        assert_eq!(jobs.take_deliverable("root", "Main")[0].result.as_deref(), Some("after interrupt"));
    }

    #[tokio::test]
    async fn steering_wins_against_settlement_without_consuming_the_job() {
        let jobs = Arc::new(JobManager::default());
        let ladders = Arc::new(Mutex::new(WaitLadders::default()));
        let (first, _) = jobs.register("root", "Main", "First", "one");
        let (_second, _) = jobs.register("root", "Main", "Second", "two");
        let (steer, steered) = tokio::sync::oneshot::channel::<()>();
        let task = {
            let jobs = jobs.clone();
            tokio::spawn(async move {
                wait_with_updates(&jobs, &ladders, "root", "Main", pending(),
                    async { let _ = steered.await; }, |_| {}).await.unwrap()
            })
        };
        tokio::task::yield_now().await;
        jobs.settle(&first, Ok("preserved result".into()));
        steer.send(()).unwrap();
        let output = task.await.unwrap();
        assert_eq!(output.structured_content.unwrap()["interrupted"], true);
        let delivered = jobs.take_deliverable("root", "Main");
        assert_eq!(delivered.len(), 1);
        assert_eq!(delivered[0].result.as_deref(), Some("preserved result"));
    }

    #[test]
    fn child_wait_snapshots_survive_heartbeats_and_clear_on_exit() {
        let jobs = JobManager::default();
        let (parent, _) = jobs.register("root", "Main", "Parent", "work");
        jobs.report_progress(&parent, json!({ "status": "running", "requests": 1 }));
        {
            let _guard = ChildWaitProgress { jobs: &jobs, root: "root", owner: "Parent" };
            jobs.report_wait_progress("root", "Parent", Some(json!({
                "op": "wait", "jobs": [{ "id": "Child", "status": "running" }],
            })));
            jobs.report_progress(&parent, json!({ "status": "running", "requests": 2 }));
            let progress = jobs.get(&parent).unwrap().progress.unwrap();
            assert_eq!(progress["requests"], 2);
            assert_eq!(progress["wait"]["jobs"][0]["id"], "Child");
        }
        assert!(jobs.get(&parent).unwrap().progress.unwrap().get("wait").is_none());
    }
}
