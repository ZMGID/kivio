//! Owner-scoped OMP `wait` for subagent jobs.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::future::Future;
use std::time::Duration;
use tokio::time::Instant;

use serde_json::{json, Value};

use crate::mcp::native_registry::NativeToolFuture;
use crate::mcp::types::{ChatToolDefinition, McpToolCallResult};

use super::jobs::{JobManager, JobSnapshot, JobStatus};
use super::{prompts, SubAgentCallCtx, WAIT_TOOL_NAME};

const WAIT_LIMIT: Duration = Duration::from_secs(300);
const SETTLE_WINDOW: Duration = Duration::from_millis(1500);
const NOTHING_TO_WAIT_FOR: &str =
    "Nothing to wait for: no background subagent job you started is running.";

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

pub fn dispatch(ctx: SubAgentCallCtx<'_>) -> NativeToolFuture<'_> {
    Box::pin(async move {
        if !ctx
            .arguments
            .as_object()
            .is_some_and(|args| args.is_empty())
        {
            return Err("wait accepts no arguments".into());
        }
        let root = ctx.native_ctx.conversation_id.as_str();
        let owner = ctx.state.sub_agents.owner_for_run(&ctx.native_ctx.run_id);
        let child_conversation = ctx
            .state
            .sub_agents
            .registry
            .get(root, &owner)
            .map(|agent| agent.child_conversation_id);
        let generation_conversation = child_conversation.as_deref().unwrap_or(root);
        let cancelled = ctx
            .state
            .chat_runtime()
            .wait_for_generation_inactive(generation_conversation, ctx.native_ctx.generation);
        let steering = ctx
            .state
            .chat_runtime()
            .wait_for_steering(generation_conversation);
        let _child_progress = (owner != super::MAIN_AGENT_ID).then(|| ChildWaitProgress {
            jobs: &ctx.state.sub_agents.jobs,
            root,
            owner: &owner,
        });
        let on_progress = |snapshots: &[JobSnapshot]| {
            if owner != super::MAIN_AGENT_ID {
                ctx.state.sub_agents.jobs.report_wait_progress(
                    root,
                    &owner,
                    Some(json!({
                        "op": "wait", "jobs": snapshots,
                    })),
                );
            }
            let Some(id) = ctx.native_ctx.tool_call_id.as_deref() else {
                return;
            };
            let Some(mut tool) = ctx
                .state
                .chat_protocol()
                .running_snapshot(root, &ctx.native_ctx.run_id, &ctx.native_ctx.message_id)
                .and_then(|snapshot| {
                    snapshot
                        .tools
                        .into_iter()
                        .find(|tool| tool.id == id && tool.round == ctx.native_ctx.round)
                })
            else {
                return;
            };
            tool.structured_content = Some(json!({ "op": "wait", "jobs": snapshots }));
            crate::chat::protocol::emit_run_event(
                ctx.app,
                &ctx.native_ctx.run_id,
                crate::chat::protocol::ChatRunEvent::ToolUpdated { tool },
            );
        };
        let job_wait = wait_for_jobs(&ctx.state.sub_agents.jobs, root, &owner, &on_progress);
        coordinated_wait(
            ctx.app,
            &ctx.state.sub_agents,
            root,
            &owner,
            job_wait,
            cancelled,
            steering,
        )
        .await
    })
}

async fn coordinated_wait(
    app: &tauri::AppHandle,
    manager: &super::SubAgentManager,
    root: &str,
    owner: &str,
    job_wait: impl Future<Output = McpToolCallResult>,
    cancelled: impl Future<Output = ()>,
    steering: impl Future<Output = ()>,
) -> Result<McpToolCallResult, String> {
    wait_for_events(manager, root, owner, job_wait, cancelled, steering, CoordinationIo {
        services: super::services::subscribe(app),
        service_ids: super::services::running(app, root, owner)?,
        completed_services: |ids: &[String]| super::services::completed(app, root, owner, ids),
        persist_message: |message| async move { super::irc::persist_message(app, &message).await },
    }).await
}

struct CoordinationIo<C, P> {
    services: tokio::sync::watch::Receiver<u64>,
    service_ids: Vec<String>,
    completed_services: C,
    persist_message: P,
}

async fn wait_for_events<C, P, F>(
    manager: &super::SubAgentManager,
    root: &str,
    owner: &str,
    job_wait: impl Future<Output = McpToolCallResult>,
    cancelled: impl Future<Output = ()>,
    steering: impl Future<Output = ()>,
    mut io: CoordinationIo<C, P>,
) -> Result<McpToolCallResult, String>
where
    C: Fn(&[String]) -> Result<Vec<Value>, String>,
    P: Fn(super::irc::Message) -> F,
    F: Future<Output = Result<(), String>>,
{
    let mut incoming = manager.irc.subscribe();
    let mut changed_jobs = manager.jobs.subscribe();
    let mut changed_awaits = manager.awaits.subscribe();
    let jobs = manager.jobs.running(root, owner);
    let mut has_jobs = !jobs.is_empty() || !manager.jobs.undelivered(root, owner).is_empty();
    let mut asked = manager.irc.pending_replies(root, owner);
    let mut targets: Vec<_> = jobs.iter().map(|job| job.agent_id.clone()).collect();
    targets.extend(asked.iter().map(|debt| debt.target.clone()));
    let _hold = io.service_ids.is_empty().then(|| {
        manager
            .awaits
            .hold(root, owner, targets, manager.irc.mark(), true)
    });
    tokio::pin!(job_wait);
    tokio::pin!(cancelled);
    tokio::pin!(steering);
    let deadline = tokio::time::sleep(WAIT_LIMIT);
    tokio::pin!(deadline);
    loop {
        let asked_targets: Vec<_> = asked.iter().map(|debt| debt.target.clone()).collect();
        let parent = manager
            .registry
            .get(root, owner)
            .map(|record| record.parent_id);
        let mut claim = manager.irc.claim(root, owner, |message| {
            (!message.progress || parent.as_ref() == Some(&message.from))
                && (!message.broadcast || asked_targets.contains(&message.from))
        });
        if !claim.messages().is_empty() {
            let messages = claim.messages().to_vec();
            if owner == super::MAIN_AGENT_ID {
                for message in &messages {
                    (io.persist_message)(message.clone()).await?;
                }
            }
            let mut output = result(
                messages
                    .iter()
                    .map(|message| format!("{}: {}", message.from, message.body))
                    .collect::<Vec<_>>()
                    .join("\n\n"),
                &[],
            );
            output.raw["messages"] = json!(messages);
            output.raw["useless"] = json!(false);
            output.structured_content = Some(output.raw.clone());
            claim.commit();
            return Ok(output);
        }
        claim.commit();
        drop(claim);
        let completed = (io.completed_services)(&io.service_ids)?;
        if !completed.is_empty() {
            let mut output = result("Background service finished.".into(), &[]);
            output.raw["services"] = json!(completed);
            output.raw["useless"] = json!(false);
            output.structured_content = Some(output.raw.clone());
            return Ok(output);
        }
        asked = manager.irc.pending_replies(root, owner);
        let mut blocked = Vec::new();
        asked.retain(|debt| {
            let is_blocked = manager
                .awaits
                .blocks_on(root, &debt.target, owner, debt.sequence)
                || manager
                    .awaits
                    .blocks_on(root, &debt.target, "@user", debt.sequence);
            let alive = debt.target == super::MAIN_AGENT_ID
                || manager
                    .registry
                    .get(root, &debt.target)
                    .is_some_and(|record| {
                        record.status == super::registry::AgentStatus::Running
                            || (record.status != super::registry::AgentStatus::Aborted
                                && (debt.promised || manager.irc.has_pending(root, &debt.target)))
                    });
            if is_blocked {
                blocked.push(debt.target.clone());
            }
            if !alive || is_blocked {
                manager.irc.release_debt(root, owner, &debt.target);
                false
            } else {
                true
            }
        });
        has_jobs |= !manager.jobs.running(root, owner).is_empty()
            || !manager.jobs.undelivered(root, owner).is_empty();
        if !has_jobs && io.service_ids.is_empty() && asked.is_empty() {
            return Ok(result(
                if blocked.is_empty() {
                    NOTHING_TO_WAIT_FOR.into()
                } else {
                    format!(
                        "Cannot wait for agents blocked on this caller or the user: {}.",
                        blocked.join(", ")
                    )
                },
                &[],
            ));
        }
        tokio::select! {
            biased;
            _ = &mut cancelled => return Err("cancelled".into()),
            _ = &mut steering => return Ok(steering_result()),
            changed = incoming.changed() => if changed.is_err() { return Err("Agent message bus stopped".into()); },
            changed = changed_jobs.changed() => if changed.is_err() { return Err("Subagent job manager stopped".into()); },
            changed = changed_awaits.changed() => if changed.is_err() { return Err("Agent wait graph stopped".into()); },
            changed = io.services.changed() => if changed.is_err() { return Err("Background service manager stopped".into()); },
            output = &mut job_wait, if has_jobs => return Ok(output),
            _ = &mut deadline => return Ok(result("No result or reply arrived before the wait limit. Background updates arrive automatically.".into(), &[])),
        }
    }
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

fn steering_result() -> McpToolCallResult {
    let mut output = result("Wait interrupted by user steering.".into(), &[]);
    output.raw["useless"] = json!(false);
    output.raw["interrupted"] = json!(true);
    output.structured_content = Some(output.raw.clone());
    output
}

async fn wait_for_jobs(
    jobs: &JobManager,
    root: &str,
    owner: &str,
    on_progress: &(impl Fn(&[JobSnapshot]) + Sync),
) -> McpToolCallResult {
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
    let progress_interval = Duration::from_millis(500);
    let mut progress =
        tokio::time::interval_at(Instant::now() + progress_interval, progress_interval);
    let mut settle_deadline = None;
    loop {
        let mut any_running = false;
        let mut any_settled = false;
        for id in &watched.ids {
            match jobs.status(id) {
                Some(JobStatus::Running) => any_running = true,
                _ => any_settled = true,
            }
        }
        if any_settled && settle_deadline.is_none() {
            settle_deadline = Some(Instant::now() + SETTLE_WINDOW);
        }
        if !any_running {
            break;
        }
        tokio::select! {
            _ = async {
                match settle_deadline {
                    Some(until) => tokio::time::sleep_until(until).await,
                    None => std::future::pending().await,
                }
            } => break,
            _ = progress.tick() => emit_progress(),
            changed = changed.changed() => {
                if changed.is_err() {
                    break;
                }
            }
        }
    }
    snapshot(jobs, Vec::new(), &watched.ids)
}

#[derive(Clone, Copy)]
enum Delivery {
    Recovered,
    Direct,
    AlreadyConsumed,
    None,
}

fn snapshot(
    jobs: &JobManager,
    claimed: Vec<JobSnapshot>,
    watched_ids: &[String],
) -> McpToolCallResult {
    let mut rows: Vec<(JobSnapshot, Delivery)> = claimed
        .into_iter()
        .map(|job| (job, Delivery::Recovered))
        .collect();
    let mut claimed: HashMap<_, _> = jobs
        .take_wait_results(watched_ids)
        .into_iter()
        .map(|job| (job.id.clone(), job))
        .collect();
    for id in watched_ids {
        if let Some(job) = claimed.remove(id) {
            rows.push((job, Delivery::Direct));
        } else if let Some(job) = jobs.get(id) {
            match job.status {
                JobStatus::Running | JobStatus::Cancelled => rows.push((job, Delivery::None)),
                _ if job.consumed => rows.push((job, Delivery::AlreadyConsumed)),
                _ => {
                    // It settled after the batch claim; claim it before unwatching.
                    if let Some(job) = jobs
                        .take_wait_results(std::slice::from_ref(id))
                        .into_iter()
                        .next()
                    {
                        rows.push((job, Delivery::Direct));
                    } else if let Some(job) = jobs.get(id).filter(|job| job.consumed) {
                        rows.push((job, Delivery::AlreadyConsumed));
                    }
                }
            }
        }
    }

    let mut text = String::new();
    let completed = rows
        .iter()
        .filter(|(job, _)| job.status != JobStatus::Running)
        .count();
    if completed > 0 {
        write!(text, "## Completed ({completed})\n").unwrap();
        for (job, delivery) in rows
            .iter()
            .filter(|(job, _)| job.status != JobStatus::Running)
        {
            let status = match job.status {
                JobStatus::Completed => "completed",
                JobStatus::Failed => "failed",
                JobStatus::Cancelled => "cancelled",
                JobStatus::Running => unreachable!(),
            };
            write!(
                text,
                "\n### {} [task] — {status}\nLabel: {}\n",
                job.id, job.label
            )
            .unwrap();
            if job.id != job.agent_id {
                if job.status == JobStatus::Completed {
                    writeln!(
                        text,
                        "Agent: agent://{}, history://{}",
                        job.agent_id, job.agent_id
                    )
                    .unwrap();
                } else {
                    writeln!(text, "Agent: history://{}", job.agent_id).unwrap();
                }
            }
            match delivery {
                Delivery::Recovered => writeln!(
                    text,
                    "Delivery: not auto-delivered; recovered by this snapshot."
                )
                .unwrap(),
                Delivery::Direct => {
                    writeln!(text, "Delivery: delivered directly to wait.").unwrap()
                }
                Delivery::AlreadyConsumed => {
                    writeln!(text, "Delivery: already delivered or recovered.").unwrap()
                }
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
        for (job, _) in rows
            .iter()
            .filter(|(job, _)| job.status == JobStatus::Running)
        {
            write!(
                text,
                "\n- `{}` [task] — {}{}",
                job.id,
                job.label,
                progress_suffix(job)
            )
            .unwrap();
        }
    }
    if text.is_empty() {
        text.push_str("No background jobs.");
    }
    let snapshots: Vec<_> = rows.iter().map(|(job, _)| job).collect();
    result(text.trim_end().to_string(), &snapshots)
}

fn progress_suffix(job: &JobSnapshot) -> String {
    let Some(progress) = job.progress.as_ref() else {
        return String::new();
    };
    let mut parts = Vec::new();
    if let Some(requests) = progress
        .get("requests")
        .and_then(Value::as_u64)
        .filter(|n| *n > 0)
    {
        parts.push(format!("{requests} req"));
    }
    if let Some(cost) = progress
        .get("cost")
        .and_then(Value::as_f64)
        .filter(|n| *n > 0.0)
    {
        parts.push(if cost < 0.005 {
            "<$0.01".into()
        } else {
            format!("${cost:.2}")
        });
    }
    if let Some(tokens) = progress
        .get("contextTokens")
        .and_then(Value::as_u64)
        .filter(|n| *n > 0)
    {
        let mut context = format!("ctx {}", format_number(tokens));
        if let Some(window) = progress
            .get("contextWindow")
            .and_then(Value::as_u64)
            .filter(|n| *n > 0)
        {
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
        format!(
            " — {} · {}",
            format_duration(job.duration_ms().max(0) as u64),
            parts.join(" · ")
        )
    }
}

fn format_number(value: u64) -> String {
    let (divisor, suffix) = if value >= 1_000_000 {
        (1_000_000, "M")
    } else if value >= 1_000 {
        (1_000, "K")
    } else {
        return value.to_string();
    };
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
        if remainder == 0 {
            format!("{}m", seconds / 60)
        } else {
            format!("{}m{remainder}s", seconds / 60)
        }
    } else {
        let minutes = (seconds % 3_600) / 60;
        if minutes == 0 {
            format!("{}h", seconds / 3_600)
        } else {
            format!("{}h{minutes}m", seconds / 3_600)
        }
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
    use super::super::SubAgentManager;
    use super::*;
    use std::future::pending;
    use std::sync::{Arc, Mutex};

    fn peer_message(
        sequence: u64,
        from: &str,
        to: &str,
        progress: bool,
    ) -> super::super::irc::Message {
        super::super::irc::Message {
            id: sequence.to_string(),
            root: "root".into(),
            from: from.into(),
            to: to.into(),
            body: format!("message {sequence}"),
            sequence,
            reply_to: None,
            progress,
            broadcast: false,
            wake_relay: false,
            timestamp: 0,
        }
    }

    fn seed_reply_wait(manager: &SubAgentManager) {
        manager.registry.register(
            serde_json::from_value(json!({
                "id":"Peer", "root":"root", "parentId":"Main", "generation":0,
                "displayName":"Peer", "agentType":"task", "status":"running", "depth":1,
                "childConversationId":"subagent-root-Peer", "runId":null,
                "sessionFile":"Peer.jsonl", "outputFile":"Peer.md", "lastActivity":0,
                "activity":null, "published":true,
            }))
            .unwrap(),
        );
        manager
            .irc
            .seed_message(peer_message(1, "Main", "Peer", false));
    }

    async fn test_coordinated_wait(
        manager: &SubAgentManager,
        cancelled: impl Future<Output = ()>,
        steering: impl Future<Output = ()>,
        on_progress: impl Fn(&[JobSnapshot]) + Sync,
    ) -> Result<McpToolCallResult, String> {
        let (_services, changed) = tokio::sync::watch::channel(0);
        wait_for_events(
            manager,
            "root",
            "Main",
            wait_for_jobs(&manager.jobs, "root", "Main", &on_progress),
            cancelled,
            steering,
            CoordinationIo {
                services: changed,
                service_ids: Vec::new(),
                completed_services: |_: &[String]| Ok(Vec::new()),
                persist_message: |_| std::future::ready(Ok(())),
            },
        )
        .await
    }

    #[tokio::test(start_paused = true)]
    async fn wait_stays_pending_until_background_work_finishes() {
        let manager = SubAgentManager::default();
        let (job, _) = manager.jobs.register("root", "Main", "Scout", "research");
        let mut waiting = Box::pin(test_coordinated_wait(
            &manager,
            pending(),
            pending(),
            |_| {},
        ));
        assert!(futures::poll!(&mut waiting).is_pending());

        tokio::time::advance(Duration::from_secs(120)).await;
        assert!(
            futures::poll!(&mut waiting).is_pending(),
            "an idle timeout must not wake the model while work continues"
        );

        manager.jobs.settle(&job, Ok("research finished".into()));
        let output = waiting.await.unwrap();
        assert!(output.content.contains("research finished"));
        assert!(manager.jobs.take_deliverable("root", "Main").is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn reply_wait_observes_promoted_jobs_without_an_irc_relay() {
        for settled_before_observation in [false, true] {
            let manager = SubAgentManager::default();
            seed_reply_wait(&manager);
            let (job, _) = manager.jobs.register_inline("root", "Main", "Peer", "wake");
            let mut waiting = Box::pin(test_coordinated_wait(
                &manager,
                pending(),
                pending(),
                |_| {},
            ));
            assert!(futures::poll!(&mut waiting).is_pending());

            for (root, owner) in [("other-root", "Main"), ("root", "Other")] {
                let (private, _) = manager.jobs.register(root, owner, "Private", "unrelated");
                manager.jobs.settle(&private, Ok("private result".into()));
            }
            assert!(futures::poll!(&mut waiting).is_pending());
            if settled_before_observation {
                manager.jobs.settle(&job, Ok("promoted result".into()));
            }
            assert!(manager.jobs.promote_inline("root", &job));
            if !settled_before_observation {
                assert!(futures::poll!(&mut waiting).is_pending());
                manager.jobs.settle(&job, Ok("promoted result".into()));
            }
            let std::task::Poll::Ready(output) = futures::poll!(&mut waiting) else {
                panic!("a newly owned job must finish the reply wait");
            };
            let output = output.unwrap();
            assert!(output.content.contains("promoted result"));
            assert!(!output.content.contains("private result"));
            assert_eq!(output.raw["useless"], false);
            assert!(output.raw.get("messages").is_none());
            assert!(!manager.irc.has_pending("root", "Main"));
            assert!(manager.jobs.take_deliverable("root", "Main").is_empty());
            assert_eq!(manager.jobs.take_deliverable("root", "Other").len(), 1);
            assert_eq!(manager.jobs.take_deliverable("other-root", "Main").len(), 1);
        }
    }

    #[tokio::test(start_paused = true)]
    async fn reply_wait_preserves_progress_promises_and_stays_quiet() {
        let manager = SubAgentManager::default();
        seed_reply_wait(&manager);
        let mut question = manager.irc.claim("root", "Peer", |_| true);
        question.commit();
        drop(question);
        manager
            .irc
            .seed_message(peer_message(2, "Peer", "Main", true));
        manager
            .registry
            .set_status("root", "Peer", super::super::registry::AgentStatus::Idle);
        let changed = manager.irc.subscribe();
        let mut waiting = Box::pin(test_coordinated_wait(
            &manager,
            pending(),
            pending(),
            |_| {},
        ));
        assert!(futures::poll!(&mut waiting).is_pending());
        assert!(!changed.has_changed().unwrap());
        assert!(manager.irc.pending_replies("root", "Main")[0].promised);
        assert!(manager.irc.has_pending("root", "Main"));

        let (unrelated, _) = manager.jobs.register("root", "Other", "Other", "unrelated");
        manager.jobs.settle(&unrelated, Ok("private result".into()));
        assert!(futures::poll!(&mut waiting).is_pending());
        assert!(
            !changed.has_changed().unwrap(),
            "an empty claim must not wake its own wait"
        );
        assert!(manager.irc.pending_replies("root", "Main")[0].promised);

        manager
            .irc
            .seed_message(peer_message(3, "Peer", "Main", false));
        let std::task::Poll::Ready(output) = futures::poll!(&mut waiting) else {
            panic!("the actual reply must wake the wait");
        };
        assert_eq!(output.unwrap().raw["messages"][0]["id"], "3");
        assert!(manager.irc.pending_replies("root", "Main").is_empty());
        let progress = manager.irc.claim("root", "Main", |_| true);
        assert_eq!(progress.messages().len(), 1);
        assert!(progress.messages()[0].progress);
    }

    #[tokio::test(start_paused = true)]
    async fn promoted_jobs_release_watchers_after_reply_wait_cancellation_or_drop() {
        for cancel in [false, true] {
            let manager = SubAgentManager::default();
            seed_reply_wait(&manager);
            let (first, _) = manager.jobs.register_inline("root", "Main", "First", "one");
            let (second, _) = manager
                .jobs
                .register_inline("root", "Main", "Second", "two");
            let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel::<()>();
            let mut waiting = Box::pin(test_coordinated_wait(
                &manager,
                async {
                    let _ = cancel_rx.await;
                },
                pending(),
                |_| {},
            ));
            assert!(futures::poll!(&mut waiting).is_pending());
            manager.jobs.promote_inline("root", &first);
            manager.jobs.promote_inline("root", &second);
            assert!(futures::poll!(&mut waiting).is_pending());
            manager.jobs.settle(&first, Ok("preserved result".into()));
            assert!(futures::poll!(&mut waiting).is_pending());
            assert!(manager.jobs.take_deliverable("root", "Main").is_empty());
            if cancel {
                cancel_tx.send(()).unwrap();
                let std::task::Poll::Ready(output) = futures::poll!(&mut waiting) else {
                    panic!("cancellation must stop an active reply wait");
                };
                assert_eq!(output.unwrap_err(), "cancelled");
            }
            drop(waiting);
            let delivered = manager.jobs.take_deliverable("root", "Main");
            assert_eq!(delivered.len(), 1);
            assert_eq!(delivered[0].result.as_deref(), Some("preserved result"));
            manager.jobs.settle(&second, Ok("later result".into()));
            assert_eq!(manager.jobs.take_deliverable("root", "Main").len(), 1);
            assert!(manager.jobs.take_deliverable("root", "Main").is_empty());
        }
    }

    #[tokio::test(start_paused = true)]
    async fn reply_wait_without_jobs_is_interrupted_by_user_steering() {
        let manager = SubAgentManager::default();
        seed_reply_wait(&manager);
        let (steer, steered) = tokio::sync::oneshot::channel::<()>();
        let mut waiting = Box::pin(test_coordinated_wait(
            &manager,
            pending(),
            async {
                let _ = steered.await;
            },
            |_| {},
        ));
        assert!(futures::poll!(&mut waiting).is_pending());
        steer.send(()).unwrap();
        let std::task::Poll::Ready(output) = futures::poll!(&mut waiting) else {
            panic!("user steering must interrupt a reply-only wait");
        };
        let output = output.unwrap();
        assert_eq!(output.raw["interrupted"], true);
        assert_eq!(output.raw["useless"], false);
        assert_eq!(manager.irc.pending_replies("root", "Main").len(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn child_reply_wait_ends_when_main_starts_waiting_on_user() {
        let manager = SubAgentManager::default();
        manager
            .irc
            .seed_message(peer_message(1, "Peer", "Main", false));
        let (_services, changed) = tokio::sync::watch::channel(0);
        let mut waiting = Box::pin(wait_for_events(
            &manager,
            "root",
            "Peer",
            pending(),
            pending(),
            pending(),
            CoordinationIo {
                services: changed,
                service_ids: Vec::new(),
                completed_services: |_: &[String]| Ok(Vec::new()),
                persist_message: |_| std::future::ready(Ok(())),
            },
        ));
        assert!(futures::poll!(&mut waiting).is_pending());

        let _hold = manager.awaits.hold(
            "root",
            "Main",
            vec!["@user".into()],
            manager.irc.mark(),
            false,
        );
        let std::task::Poll::Ready(output) = futures::poll!(&mut waiting) else {
            panic!("an asked agent becoming blocked on the user must wake the reply wait");
        };
        assert!(output
            .unwrap()
            .content
            .contains("Cannot wait for agents blocked on this caller or the user: Main"));
    }

    #[test]
    fn definition_has_no_parameters() {
        let tool = definition();
        assert_eq!(tool.name, "wait");
        assert_eq!(tool.description, prompts::WAIT_TOOL);
        assert_eq!(
            tool.input_schema,
            json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            })
        );
    }

    #[tokio::test]
    async fn wait_returns_undelivered_first_and_consumes_once() {
        let manager = SubAgentManager::default();
        let jobs = &manager.jobs;
        let (done, _) = jobs.register("root", "Main", "Done", "completed work");
        let (running, _) = jobs.register("root", "Main", "Running", "ongoing work");
        let (other, _) = jobs.register("root", "Other", "Private", "private work");
        jobs.settle(&done, Ok("unique result".into()));
        jobs.settle(&other, Ok("private result".into()));
        let output = tokio::time::timeout(
            Duration::from_millis(100),
            test_coordinated_wait(&manager, pending(), pending(), |_| {}),
        )
        .await
        .expect("undelivered result must return without waiting")
        .unwrap();
        assert!(output.content.contains("## Completed (1)"));
        assert!(output.content.contains("unique result"));
        assert!(output.content.contains("## Still Running (1)"));
        assert!(!output.content.contains("private result"));
        assert_eq!(
            output.structured_content.as_ref().unwrap()["jobs"][0]["result"],
            "unique result"
        );
        assert_eq!(
            output.structured_content.as_ref().unwrap()["useless"],
            false
        );
        assert!(jobs.take_deliverable("root", "Main").is_empty());
        assert!(jobs.cancel("root", &running, "Main"));
        let second = test_coordinated_wait(&manager, pending(), pending(), |_| {})
            .await
            .unwrap();
        assert_eq!(second.content, NOTHING_TO_WAIT_FOR);
        assert_eq!(second.structured_content.as_ref().unwrap()["useless"], true);
        assert!(!second.content.contains("unique result"));
        assert_eq!(jobs.take_deliverable("root", "Other").len(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn wait_collects_nearby_settlements_and_stops_when_all_settle() {
        let manager = Arc::new(SubAgentManager::default());
        let jobs = &manager.jobs;
        let (first, _) = jobs.register("root", "Main", "First", "one");
        let (second, _) = jobs.register("root", "Main", "Second", "two");
        let task_manager = manager.clone();
        let task = tokio::spawn(async move {
            test_coordinated_wait(&task_manager, pending(), pending(), |_| {})
                .await
                .unwrap()
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
        let manager = Arc::new(SubAgentManager::default());
        let jobs = &manager.jobs;
        let (first, _) = jobs.register("root", "Main", "First", "one");
        let (second, _) = jobs.register("root", "Main", "Second", "two");
        let task_manager = manager.clone();
        let task = tokio::spawn(async move {
            test_coordinated_wait(&task_manager, pending(), pending(), |_| {})
                .await
                .unwrap()
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
    async fn wait_limit_leaves_unfinished_work_available_for_auto_delivery() {
        let manager = SubAgentManager::default();
        let (job, _) = manager.jobs.register("root", "Main", "Scout", "ongoing");
        let mut waiting = Box::pin(test_coordinated_wait(
            &manager,
            pending(),
            pending(),
            |_| {},
        ));
        assert!(futures::poll!(&mut waiting).is_pending());

        tokio::time::advance(Duration::from_secs(300)).await;
        let output = tokio::time::timeout(Duration::from_millis(10), waiting)
            .await
            .expect("wait must return at the five-minute limit")
            .unwrap();
        assert_eq!(output.raw["useless"], true);
        assert_eq!(manager.jobs.get(&job).unwrap().status, JobStatus::Running);
        manager.jobs.settle(&job, Ok("later result".into()));
        assert_eq!(
            manager.jobs.take_deliverable("root", "Main")[0]
                .result
                .as_deref(),
            Some("later result")
        );
    }

    #[tokio::test]
    async fn cancellation_during_collection_preserves_completed_result() {
        let manager = Arc::new(SubAgentManager::default());
        let jobs = &manager.jobs;
        let (first, _) = jobs.register("root", "Main", "First", "one");
        let (second, _) = jobs.register("root", "Main", "Second", "two");
        let (cancel, cancelled) = tokio::sync::oneshot::channel::<()>();
        let task_manager = manager.clone();
        let task = tokio::spawn(async move {
            test_coordinated_wait(
                &task_manager,
                async {
                    let _ = cancelled.await;
                },
                pending(),
                |_| {},
            )
            .await
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
        let manager = Arc::new(SubAgentManager::default());
        let jobs = &manager.jobs;
        let (first, _) = jobs.register("root", "Main", "First", "one");
        let (cancel, cancelled) = tokio::sync::oneshot::channel::<()>();
        let task_manager = manager.clone();
        let task = tokio::spawn(async move {
            test_coordinated_wait(
                &task_manager,
                async {
                    let _ = cancelled.await;
                },
                pending(),
                |_| {},
            )
            .await
        });
        tokio::task::yield_now().await;
        cancel.send(()).unwrap();
        assert_eq!(task.await.unwrap().unwrap_err(), "cancelled");
        jobs.settle(&first, Ok("after cancellation".into()));
        assert_eq!(jobs.take_deliverable("root", "Main").len(), 1);

        let (second, _) = jobs.register("root", "Main", "Second", "two");
        let task_manager = manager.clone();
        let task = tokio::spawn(async move {
            test_coordinated_wait(&task_manager, pending(), pending(), |_| {}).await
        });
        tokio::task::yield_now().await;
        task.abort();
        let _ = task.await;
        jobs.settle(&second, Ok("after drop".into()));
        assert_eq!(jobs.take_deliverable("root", "Main").len(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn wait_emits_periodic_non_consuming_snapshots_and_cancellation_releases_them() {
        let manager = Arc::new(SubAgentManager::default());
        let jobs = &manager.jobs;
        let updates = Arc::new(Mutex::new(Vec::<Vec<JobSnapshot>>::new()));
        let (job, _) = jobs.register("root", "Main", "Scout", "inspect");
        let (cancel, cancelled) = tokio::sync::oneshot::channel::<()>();
        let task = {
            let manager = manager.clone();
            let updates = updates.clone();
            tokio::spawn(async move {
                test_coordinated_wait(
                    &manager,
                    async {
                        let _ = cancelled.await;
                    },
                    pending(),
                    |snapshot| updates.lock().unwrap().push(snapshot.to_vec()),
                )
                .await
            })
        };
        tokio::task::yield_now().await;
        assert_eq!(updates.lock().unwrap().len(), 1);
        jobs.report_progress(&job, json!({ "stage": "reading" }));
        tokio::time::advance(Duration::from_millis(500)).await;
        tokio::task::yield_now().await;
        assert_eq!(
            updates.lock().unwrap().last().unwrap()[0]
                .progress
                .as_ref()
                .unwrap()["stage"],
            "reading"
        );
        cancel.send(()).unwrap();
        assert_eq!(task.await.unwrap().unwrap_err(), "cancelled");
        jobs.settle(&job, Ok("after interrupt".into()));
        assert_eq!(
            jobs.take_deliverable("root", "Main")[0].result.as_deref(),
            Some("after interrupt")
        );
    }

    #[tokio::test]
    async fn steering_wins_against_settlement_without_consuming_the_job() {
        let manager = Arc::new(SubAgentManager::default());
        let jobs = &manager.jobs;
        let (first, _) = jobs.register("root", "Main", "First", "one");
        let (_second, _) = jobs.register("root", "Main", "Second", "two");
        let (steer, steered) = tokio::sync::oneshot::channel::<()>();
        let task = {
            let manager = manager.clone();
            tokio::spawn(async move {
                test_coordinated_wait(
                    &manager,
                    pending(),
                    async {
                        let _ = steered.await;
                    },
                    |_| {},
                )
                .await
                .unwrap()
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
            let _guard = ChildWaitProgress {
                jobs: &jobs,
                root: "root",
                owner: "Parent",
            };
            jobs.report_wait_progress(
                "root",
                "Parent",
                Some(json!({
                    "op": "wait", "jobs": [{ "id": "Child", "status": "running" }],
                })),
            );
            jobs.report_progress(&parent, json!({ "status": "running", "requests": 2 }));
            let progress = jobs.get(&parent).unwrap().progress.unwrap();
            assert_eq!(progress["requests"], 2);
            assert_eq!(progress["wait"]["jobs"][0]["id"], "Child");
        }
        assert!(jobs
            .get(&parent)
            .unwrap()
            .progress
            .unwrap()
            .get("wait")
            .is_none());
    }
}
