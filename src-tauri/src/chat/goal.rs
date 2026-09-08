use serde::Deserialize;
use serde_json::Value;
use tauri::{AppHandle, Manager, State};
use uuid::Uuid;

use crate::chat::model::ModelUsage;
use crate::chat::types::{Conversation, GoalCriterion, GoalState, GoalStatus};
use crate::mcp::native_registry::NativeToolFuture;
use crate::mcp::registry::NativeToolContext;
use crate::mcp::types::McpToolCallResult;
use crate::mcp::ChatToolDefinition;
use crate::state::AppState;

mod evidence;

pub const GET_GOAL_TOOL: &str = "get_goal";
pub const INIT_GOAL_CRITERIA_TOOL: &str = "initialize_goal_criteria";
pub const REPORT_GOAL_PROGRESS_TOOL: &str = "report_goal_progress";
pub const COMPLETE_GOAL_TOOL: &str = "goal_complete";
pub const BLOCK_GOAL_TOOL: &str = "goal_blocked";
pub const WAIT_GOAL_TOOL: &str = "goal_wait";

pub fn is_goal_tool_name(name: &str) -> bool {
    matches!(
        name,
        GET_GOAL_TOOL
            | INIT_GOAL_CRITERIA_TOOL
            | REPORT_GOAL_PROGRESS_TOOL
            | COMPLETE_GOAL_TOOL
            | BLOCK_GOAL_TOOL
            | WAIT_GOAL_TOOL
    )
}

pub fn is_running(status: GoalStatus) -> bool {
    matches!(status, GoalStatus::Active | GoalStatus::Verifying)
}

pub fn start(conversation: &mut Conversation, objective: &str) -> Result<GoalState, String> {
    if !matches!(
        conversation.agent_runtime.kind,
        crate::chat::types::AgentRuntimeKind::Builtin
    ) {
        return Err("Goal mode is available only for Kivio Agent".into());
    }
    if crate::chat::plan::is_orchestrate_mode(&conversation.agent_plan_state) {
        return Err("Goal mode does not support Orchestrate in this version".into());
    }
    if conversation.reply_models.len() >= 2 {
        return Err("Goal mode requires a single reply model".into());
    }
    let objective = objective.trim();
    if objective.is_empty() {
        return Err("Goal objective cannot be empty".into());
    }
    if objective.chars().count() > 4000 {
        return Err("Goal objective is limited to 4000 characters".into());
    }
    conversation.agent_plan_state = crate::chat::plan::with_mode(
        &conversation.agent_plan_state,
        crate::chat::types::AgentPlanMode::Act,
    );
    let now = chrono::Local::now().timestamp();
    let state = GoalState {
        id: format!("goal_{}", Uuid::new_v4()),
        version: 1,
        objective: objective.to_string(),
        status: GoalStatus::Active,
        criteria: Vec::new(),
        status_reason: None,
        progress_summary: None,
        progress_revision: 0,
        last_recorded_progress_revision: 0,
        active_run_id: None,
        automatic_runs: 0,
        no_progress_runs: 0,
        last_response_fingerprint: None,
        input_tokens: None,
        output_tokens: None,
        total_tokens: None,
        completed_at: None,
        completed_message_id: None,
        created_at: now,
        updated_at: now,
    };
    conversation.goal_state = Some(state.clone());
    Ok(state)
}

pub fn format_prompt(goal: Option<&GoalState>) -> Option<String> {
    let goal = goal.filter(|g| is_running(g.status))?;
    let criteria = if goal.criteria.is_empty() {
        "No acceptance criteria have been initialized. Call initialize_goal_criteria before substantial work.".to_string()
    } else {
        goal.criteria
            .iter()
            .map(|c| {
                format!(
                    "- [{}] {}: {}{}",
                    if c.verified { "x" } else { " " },
                    c.id,
                    c.text,
                    c.evidence
                        .as_deref()
                        .map(|e| format!(" — evidence: {e}"))
                        .unwrap_or_default()
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    Some(format!(
        "Goal mode is active. Keep working autonomously toward the objective; do not stop at a plan or partial result. Use the Goal tools to maintain criteria and evidence. Only goal_complete can finish the Goal. If user input or an external event is truly required, use goal_wait. Use goal_blocked only for a concrete impasse.\n\nGoal id: {}\nGoal version: {}\nObjective: {}\n\nAcceptance criteria:\n{}\n\nLatest progress: {}",
        goal.id, goal.version, goal.objective, criteria,
        goal.progress_summary.as_deref().unwrap_or("No progress reported yet.")
    ))
}

pub fn append_tool_definitions(
    tools: &mut Vec<ChatToolDefinition>,
    goal: Option<&GoalState>,
) -> bool {
    if !goal.is_some_and(|g| is_running(g.status)) {
        return false;
    }
    for tool in tool_definitions() {
        if !tools
            .iter()
            .any(|t| t.openai_tool_name() == tool.openai_tool_name())
        {
            tools.push(tool);
        }
    }
    true
}

pub fn tool_definitions() -> Vec<ChatToolDefinition> {
    vec![
        tool(GET_GOAL_TOOL, "Read the current Goal, its exact id/version, acceptance criteria, progress, status, and usage.", serde_json::json!({"type":"object","properties":{},"additionalProperties":false}), true),
        tool(INIT_GOAL_CRITERIA_TOOL, "Initialize the active Goal's concise acceptance criteria exactly once. Criteria must be derived from the user's objective and may not expand scope.", serde_json::json!({"type":"object","properties":{"goal_id":{"type":"string"},"goal_version":{"type":"integer","minimum":1},"criteria":{"type":"array","minItems":1,"maxItems":20,"items":{"type":"string","minLength":1,"maxLength":500}}},"required":["goal_id","goal_version","criteria"],"additionalProperties":false}), false),
        tool(REPORT_GOAL_PROGRESS_TOOL, "Record meaningful progress and optionally verify acceptance criteria with concrete evidence. reference must be a successful tool-call id, source URL, artifact name/path, or current_run for a model self-check.", serde_json::json!({"type":"object","properties":{"goal_id":{"type":"string"},"goal_version":{"type":"integer","minimum":1},"summary":{"type":"string","minLength":1,"maxLength":2000},"evidence":{"type":"array","maxItems":20,"items":{"type":"object","properties":{"criterion_id":{"type":"string"},"summary":{"type":"string","minLength":1,"maxLength":2000},"kind":{"type":"string","enum":["model_self_check","tool_result","source","artifact"]},"reference":{"type":"string","minLength":1,"maxLength":2000}},"required":["criterion_id","summary","kind","reference"],"additionalProperties":false}}},"required":["goal_id","goal_version","summary"],"additionalProperties":false}), false),
        tool(COMPLETE_GOAL_TOOL, "Complete the active Goal only after every criterion has current concrete evidence. Provide an evidence-based final summary.", serde_json::json!({"type":"object","properties":{"goal_id":{"type":"string"},"goal_version":{"type":"integer","minimum":1},"summary":{"type":"string","minLength":1,"maxLength":4000}},"required":["goal_id","goal_version","summary"],"additionalProperties":false}), false),
        tool(BLOCK_GOAL_TOOL, "Stop the active Goal for a concrete impasse that cannot be resolved autonomously.", serde_json::json!({"type":"object","properties":{"goal_id":{"type":"string"},"goal_version":{"type":"integer","minimum":1},"reason":{"type":"string","minLength":1,"maxLength":1000},"evidence":{"type":"string","minLength":1,"maxLength":4000}},"required":["goal_id","goal_version","reason","evidence"],"additionalProperties":false}), false),
        tool(WAIT_GOAL_TOOL, "Pause automatic Goal work because user input, approval, or an external event is required.", serde_json::json!({"type":"object","properties":{"goal_id":{"type":"string"},"goal_version":{"type":"integer","minimum":1},"reason":{"type":"string","minLength":1,"maxLength":1000}},"required":["goal_id","goal_version","reason"],"additionalProperties":false}), false),
    ]
}

fn tool(name: &str, description: &str, input_schema: Value, read_only: bool) -> ChatToolDefinition {
    ChatToolDefinition {
        id: format!("native__{name}"),
        name: name.into(),
        description: description.into(),
        source: "native".into(),
        server_id: None,
        server_name: Some("Kivio".into()),
        input_schema,
        sensitive: false,
        annotations: Some(
            serde_json::json!({"readOnlyHint":read_only,"destructiveHint":false,"openWorldHint":false}),
        ),
        output_schema: None,
    }
}

#[derive(Deserialize)]
struct GuardArgs {
    goal_id: String,
    goal_version: u64,
}
#[derive(Deserialize)]
struct InitArgs {
    goal_id: String,
    goal_version: u64,
    criteria: Vec<String>,
}
#[derive(Deserialize)]
struct ProgressArgs {
    goal_id: String,
    goal_version: u64,
    summary: String,
    #[serde(default)]
    evidence: Vec<EvidenceArg>,
}
#[derive(Deserialize)]
struct EvidenceArg {
    criterion_id: String,
    summary: String,
    kind: String,
    reference: String,
}
#[derive(Deserialize)]
struct SummaryArgs {
    goal_id: String,
    goal_version: u64,
    summary: String,
}
#[derive(Deserialize)]
struct BlockArgs {
    goal_id: String,
    goal_version: u64,
    reason: String,
    evidence: String,
}
#[derive(Deserialize)]
struct WaitArgs {
    goal_id: String,
    goal_version: u64,
    reason: String,
}

fn require_current<'a>(
    conversation: &'a mut Conversation,
    guard: &GuardArgs,
    run_id: &str,
) -> Result<&'a mut GoalState, String> {
    let goal = conversation
        .goal_state
        .as_mut()
        .ok_or("No Goal is active")?;
    if goal.id != guard.goal_id || goal.version != guard.goal_version {
        return Err("Stale Goal id or version".into());
    }
    if !is_running(goal.status) {
        return Err("Goal is not active".into());
    }
    if run_id.is_empty() || goal.active_run_id.as_deref() != Some(run_id) {
        return Err("This run does not own the active Goal".into());
    }
    Ok(goal)
}

fn is_mutating_tool(name: &str) -> bool {
    matches!(
        name,
        "write_file"
            | "edit_file"
            | "apply_patch"
            | "delete_file"
            | "move_file"
            | "create_directory"
    ) || name.contains("write")
        || name.contains("edit")
        || name.contains("patch")
}

pub fn handle_conversation_tool_call<'a>(
    app: &'a AppHandle,
    ctx: &'a NativeToolContext,
    tool_name: &'a str,
    arguments: Value,
) -> NativeToolFuture<'a> {
    Box::pin(async move {
        if ctx.depth > 0 {
            return Err("Sub-agents cannot mutate the parent Goal".into());
        }
        let conversation_id = &ctx.conversation_id;
        let state = app.state::<AppState>();
        let live = state
            .chat_protocol
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .running_snapshot(conversation_id, &ctx.run_id, &ctx.message_id)
            .cloned();
        let mut validation_errors = Vec::new();
        let persisted = crate::chat::repository::repository(app)
            .mutate(app, conversation_id, |conversation| {
                match tool_name {
                    GET_GOAL_TOOL => {
                        if conversation.goal_state.is_none() {
                            return Err("No Goal exists".into());
                        }
                    }
                    INIT_GOAL_CRITERIA_TOOL => {
                        let a: InitArgs = serde_json::from_value(arguments.clone())
                            .map_err(|e| format!("Invalid arguments: {e}"))?;
                        let g = require_current(
                            conversation,
                            &GuardArgs {
                                goal_id: a.goal_id,
                                goal_version: a.goal_version,
                            },
                            &ctx.run_id,
                        )?;
                        if !g.criteria.is_empty() {
                            return Err("Acceptance criteria are already initialized".into());
                        }
                        let mut criteria = Vec::new();
                        for (i, text) in a.criteria.into_iter().enumerate() {
                            let text = text.trim().to_string();
                            if text.is_empty() {
                                return Err("Criterion cannot be empty".into());
                            }
                            criteria.push(GoalCriterion {
                                id: format!("c{}", i + 1),
                                text,
                                verified: false,
                                evidence: None,
                                evidence_kind: None,
                                evidence_ref: None,
                            });
                        }
                        g.criteria = criteria;
                        g.progress_revision += 1;
                        g.updated_at = chrono::Local::now().timestamp();
                    }
                    REPORT_GOAL_PROGRESS_TOOL => {
                        let a: ProgressArgs = serde_json::from_value(arguments.clone())
                            .map_err(|e| format!("Invalid arguments: {e}"))?;
                        let g = require_current(
                            conversation,
                            &GuardArgs {
                                goal_id: a.goal_id,
                                goal_version: a.goal_version,
                            },
                            &ctx.run_id,
                        )?;
                        let previous_criteria = g.criteria.clone();
                        for ev in a.evidence {
                            let c = g
                                .criteria
                                .iter_mut()
                                .find(|c| c.id == ev.criterion_id)
                                .ok_or_else(|| format!("Unknown criterion: {}", ev.criterion_id))?;
                            let evidence = ev.summary.trim().to_string();
                            let reference = ev.reference.trim().to_string();
                            c.evidence = Some(evidence);
                            c.evidence_kind = Some(ev.kind);
                            c.evidence_ref = Some(reference);
                        }
                        let summary = a.summary.trim().to_string();
                        g.progress_summary = Some(summary);
                        g.updated_at = chrono::Local::now().timestamp();
                        validation_errors = evidence::refresh(conversation, live.as_ref());
                        let g = conversation
                            .goal_state
                            .as_mut()
                            .expect("Goal was validated");
                        let has_new_evidence = g.criteria.iter().any(|criterion| {
                            criterion.verified
                                && previous_criteria
                                    .iter()
                                    .find(|old| old.id == criterion.id)
                                    .is_none_or(|old| {
                                        !old.verified
                                            || old.evidence_kind != criterion.evidence_kind
                                            || old.evidence_ref != criterion.evidence_ref
                                    })
                        });
                        if has_new_evidence {
                            g.progress_revision += 1;
                            g.no_progress_runs = 0;
                        }
                    }
                    COMPLETE_GOAL_TOOL => {
                        let a: SummaryArgs = serde_json::from_value(arguments.clone())
                            .map_err(|e| format!("Invalid arguments: {e}"))?;
                        if app
                            .state::<AppState>()
                            .has_goal_user_queue_pending(conversation_id)
                        {
                            return Err(
                                "User input is queued and must be processed before Goal completion"
                                    .into(),
                            );
                        }
                        let g = require_current(
                            conversation,
                            &GuardArgs {
                                goal_id: a.goal_id,
                                goal_version: a.goal_version,
                            },
                            &ctx.run_id,
                        )?;
                        if g.criteria.is_empty() {
                            return Err(
                                "Initialize acceptance criteria before completing the Goal".into(),
                            );
                        }
                        validation_errors = evidence::refresh(conversation, live.as_ref());
                        if live
                            .as_ref()
                            .is_some_and(|run| !run.pending_interactions.is_empty())
                        {
                            validation_errors.push(
                                "Resolve pending user input or approval before completing the Goal"
                                    .into(),
                            );
                        }
                        if !validation_errors.is_empty() {
                            return Ok(());
                        }
                        let g = conversation
                            .goal_state
                            .as_mut()
                            .expect("Goal was validated");
                        g.status = GoalStatus::Completed;
                        g.completed_at = Some(chrono::Local::now().timestamp());
                        g.completed_message_id = Some(ctx.message_id.clone());
                        g.status_reason = Some(a.summary.trim().into());
                        g.progress_summary = Some(a.summary.trim().into());
                        g.updated_at = chrono::Local::now().timestamp();
                    }
                    BLOCK_GOAL_TOOL => {
                        let a: BlockArgs = serde_json::from_value(arguments.clone())
                            .map_err(|e| format!("Invalid arguments: {e}"))?;
                        let g = require_current(
                            conversation,
                            &GuardArgs {
                                goal_id: a.goal_id,
                                goal_version: a.goal_version,
                            },
                            &ctx.run_id,
                        )?;
                        g.status = GoalStatus::Blocked;
                        g.status_reason = Some(format!(
                            "{}\n\nEvidence: {}",
                            a.reason.trim(),
                            a.evidence.trim()
                        ));
                        g.active_run_id = None;
                        g.updated_at = chrono::Local::now().timestamp();
                    }
                    WAIT_GOAL_TOOL => {
                        let a: WaitArgs = serde_json::from_value(arguments.clone())
                            .map_err(|e| format!("Invalid arguments: {e}"))?;
                        let g = require_current(
                            conversation,
                            &GuardArgs {
                                goal_id: a.goal_id,
                                goal_version: a.goal_version,
                            },
                            &ctx.run_id,
                        )?;
                        g.status = GoalStatus::Waiting;
                        g.status_reason = Some(a.reason.trim().into());
                        g.active_run_id = None;
                        g.updated_at = chrono::Local::now().timestamp();
                    }
                    _ => return Err(format!("Unknown Goal tool: {tool_name}")),
                }
                Ok(())
            })
            .await
            .map_err(crate::chat::repository::repository_error)?;
        emit_goal_state(app, &persisted);
        let goal = persisted.goal_state.clone().ok_or("No Goal exists")?;
        let mut result = goal_tool_result(&goal, tool_name);
        if !validation_errors.is_empty() {
            // Persist corrected checkboxes even when the completion request is rejected.
            // Returning Err inside repository::mutate would discard those corrections.
            result.is_error = tool_name == COMPLETE_GOAL_TOOL;
            result.content.push_str(&format!(
                "\nEvidence still required:\n{}",
                validation_errors.join("\n")
            ));
            result.raw["validationErrors"] = serde_json::json!(validation_errors);
            result.structured_content = Some(result.raw.clone());
        }
        Ok(result)
    })
}

fn goal_tool_result(goal: &GoalState, changed: &str) -> McpToolCallResult {
    let raw = serde_json::json!({"goalState":goal,"changed":changed});
    McpToolCallResult {
        content: format!(
            "Goal updated: {changed}. Status: {:?}. Version: {}",
            goal.status, goal.version
        ),
        is_error: false,
        raw: raw.clone(),
        artifacts: Vec::new(),
        structured_content: Some(raw),
        follow_up_user_messages: Vec::new(),
    }
}

pub fn emit_goal_state(app: &AppHandle, conversation: &Conversation) {
    crate::chat::protocol::emit_conversation_event(
        app,
        &conversation.id,
        conversation.revision,
        crate::chat::protocol::ChatConversationEvent::GoalUpdated {
            goal_state: conversation.goal_state.as_ref().map(Into::into),
        },
    );
}

pub async fn record_run(
    app: &AppHandle,
    conversation_id: &str,
    goal_id: &str,
    goal_version: u64,
    run_id: &str,
    assistant_message_id: &str,
    content: &str,
    usage: Option<&ModelUsage>,
    automatic: bool,
) -> Result<Conversation, String> {
    let fingerprint = normalize_fingerprint(content);
    let persisted = crate::chat::repository::repository(app)
        .mutate(app, conversation_id, |c| {
            let Some(g) = c.goal_state.as_mut() else {
                return Ok(());
            };
            if g.id != goal_id || g.version != goal_version {
                return Ok(());
            }
            for criterion in &mut g.criteria {
                if criterion.evidence_kind.as_deref() == Some("model_self_check")
                    && criterion.evidence_ref.as_deref() == Some("current_run")
                {
                    criterion.evidence_ref = Some(assistant_message_id.to_string());
                }
            }
            g.active_run_id = None;
            if automatic {
                g.automatic_runs += 1;
            }
            merge_usage(&mut g.input_tokens, usage.and_then(|u| u.input_tokens));
            merge_usage(&mut g.output_tokens, usage.and_then(|u| u.output_tokens));
            merge_usage(&mut g.total_tokens, usage.and_then(|u| u.total_tokens));
            if is_running(g.status) {
                if !automatic || g.progress_revision > g.last_recorded_progress_revision {
                    g.no_progress_runs = 0;
                } else {
                    g.no_progress_runs = next_no_progress_count(
                        g.no_progress_runs,
                        g.last_response_fingerprint.as_deref(),
                        &fingerprint,
                    );
                }
                g.last_recorded_progress_revision = g.progress_revision;
                g.last_response_fingerprint = Some(fingerprint);
                if g.no_progress_runs >= 3 {
                    g.status = GoalStatus::Paused;
                    g.status_reason = Some(
                        "Goal paused after three automatic runs with no observable progress".into(),
                    );
                }
            }
            let _ = run_id;
            g.updated_at = chrono::Local::now().timestamp();
            Ok(())
        })
        .await
        .map_err(crate::chat::repository::repository_error)?;
    emit_goal_state(app, &persisted);
    Ok(persisted)
}

pub async fn claim_run(
    app: &AppHandle,
    conversation_id: &str,
    goal_id: &str,
    goal_version: u64,
    run_id: &str,
) -> Result<Conversation, String> {
    let persisted = crate::chat::repository::repository(app)
        .mutate(app, conversation_id, |c| {
            let g = c.goal_state.as_mut().ok_or("No Goal exists")?;
            if g.id != goal_id || g.version != goal_version || !is_running(g.status) {
                return Err("The Goal changed before this run could start".into());
            }
            if g.active_run_id
                .as_deref()
                .is_some_and(|active| active != run_id)
            {
                return Err("Another run already owns this Goal".into());
            }
            g.active_run_id = Some(run_id.to_string());
            g.updated_at = chrono::Local::now().timestamp();
            Ok(())
        })
        .await
        .map_err(crate::chat::repository::repository_error)?;
    emit_goal_state(app, &persisted);
    Ok(persisted)
}

pub async fn pause_unfinished_after_restart(app: &AppHandle) -> Result<usize, String> {
    crate::chat::repository::repository(app)
        .bulk_mutate(app, |conversation| {
            let Some(goal) = conversation.goal_state.as_mut() else {
                return Ok(false);
            };
            if !is_running(goal.status) {
                return Ok(false);
            }
            goal.version += 1;
            goal.status = GoalStatus::Paused;
            goal.status_reason = Some("Paused after application restart".into());
            goal.active_run_id = None;
            goal.updated_at = chrono::Local::now().timestamp();
            Ok(true)
        })
        .await
        .map_err(crate::chat::repository::repository_error)
}

pub async fn pause_after_error(
    app: &AppHandle,
    conversation_id: &str,
    goal_id: &str,
    goal_version: u64,
    reason: &str,
) -> Result<Conversation, String> {
    let persisted = crate::chat::repository::repository(app)
        .mutate(app, conversation_id, |c| {
            let Some(g) = c.goal_state.as_mut() else {
                return Ok(());
            };
            if g.id == goal_id && g.version == goal_version && is_running(g.status) {
                g.status = GoalStatus::Paused;
                g.status_reason = Some(reason.to_string());
                g.active_run_id = None;
                g.updated_at = chrono::Local::now().timestamp();
            }
            Ok(())
        })
        .await
        .map_err(crate::chat::repository::repository_error)?;
    emit_goal_state(app, &persisted);
    Ok(persisted)
}

fn merge_usage(total: &mut Option<u64>, next: Option<u64>) {
    if let Some(n) = next {
        *total = Some(total.unwrap_or(0).saturating_add(n));
    }
}
fn normalize_fingerprint(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
fn next_no_progress_count(previous: u32, last: Option<&str>, next: &str) -> u32 {
    if next.is_empty() || last == Some(next) {
        previous.saturating_add(1)
    } else {
        0
    }
}

async fn mutate_goal(
    app: &AppHandle,
    conversation_id: &str,
    f: impl FnOnce(&mut GoalState) -> Result<(), String>,
) -> Result<Conversation, String> {
    let c = crate::chat::repository::repository(app)
        .mutate(app, conversation_id, |c| {
            let g = c.goal_state.as_mut().ok_or("No Goal exists")?;
            f(g)
        })
        .await
        .map_err(crate::chat::repository::repository_error)?;
    emit_goal_state(app, &c);
    Ok(c)
}

#[tauri::command]
pub async fn chat_get_goal(app: AppHandle, conversation_id: String) -> Result<Value, String> {
    let mut c = crate::chat::storage::load_conversation(&app, &conversation_id)?;
    crate::chat::commands::catalog::strip_transcripts_for_frontend(&mut c);
    Ok(serde_json::json!({"success":true,"goalState":c.goal_state,"conversation":c}))
}
#[tauri::command]
pub async fn chat_start_goal(
    app: AppHandle,
    conversation_id: String,
    objective: String,
) -> Result<Value, String> {
    let c = crate::chat::repository::repository(&app)
        .mutate(&app, &conversation_id, |c| {
            if c.goal_state
                .as_ref()
                .is_some_and(|g| !matches!(g.status, GoalStatus::Completed | GoalStatus::Cancelled))
            {
                return Err(
                    "An unfinished Goal already exists; edit or cancel it before starting another"
                        .into(),
                );
            }
            start(c, &objective)?;
            Ok(())
        })
        .await
        .map_err(crate::chat::repository::repository_error)?;
    emit_goal_state(&app, &c);
    Ok(response(c))
}
#[tauri::command]
pub async fn chat_edit_goal(
    app: AppHandle,
    state: State<'_, AppState>,
    conversation_id: String,
    objective: String,
) -> Result<Value, String> {
    state.cancel_chat_generation(&conversation_id);
    let obj = objective.trim().to_string();
    if obj.is_empty() {
        return Err("Goal objective cannot be empty".into());
    }
    let c = mutate_goal(&app, &conversation_id, |g| {
        let remain_paused = matches!(g.status, GoalStatus::Paused);
        g.version += 1;
        g.objective = obj;
        g.status = if remain_paused {
            GoalStatus::Paused
        } else {
            GoalStatus::Active
        };
        g.status_reason = None;
        g.criteria.clear();
        g.completed_at = None;
        g.completed_message_id = None;
        g.active_run_id = None;
        g.no_progress_runs = 0;
        g.last_response_fingerprint = None;
        g.last_recorded_progress_revision = g.progress_revision;
        g.updated_at = chrono::Local::now().timestamp();
        Ok(())
    })
    .await?;
    Ok(response(c))
}
#[tauri::command]
pub async fn chat_pause_goal(
    app: AppHandle,
    state: State<'_, AppState>,
    conversation_id: String,
) -> Result<Value, String> {
    state.cancel_chat_generation(&conversation_id);
    let c = mutate_goal(&app, &conversation_id, |g| {
        if is_running(g.status) {
            g.version += 1;
            g.status = GoalStatus::Paused;
            g.status_reason = Some("Paused by user".into());
            g.active_run_id = None;
        }
        Ok(())
    })
    .await?;
    Ok(response(c))
}
#[tauri::command]
pub async fn chat_resume_goal(app: AppHandle, conversation_id: String) -> Result<Value, String> {
    let c = mutate_goal(&app, &conversation_id, |g| {
        if !matches!(g.status, GoalStatus::Completed | GoalStatus::Cancelled) {
            g.version += 1;
            g.status = GoalStatus::Active;
            g.status_reason = None;
            g.active_run_id = None;
            g.no_progress_runs = 0;
            g.last_response_fingerprint = None;
            g.last_recorded_progress_revision = g.progress_revision;
        }
        Ok(())
    })
    .await?;
    Ok(response(c))
}
#[tauri::command]
pub async fn chat_cancel_goal(
    app: AppHandle,
    state: State<'_, AppState>,
    conversation_id: String,
) -> Result<Value, String> {
    state.cancel_chat_generation(&conversation_id);
    let c = mutate_goal(&app, &conversation_id, |g| {
        g.version += 1;
        g.status = GoalStatus::Cancelled;
        g.status_reason = Some("Cancelled by user".into());
        g.active_run_id = None;
        Ok(())
    })
    .await?;
    Ok(response(c))
}
#[tauri::command]
pub fn chat_set_goal_user_queue_pending(
    state: State<'_, AppState>,
    conversation_id: String,
    pending: bool,
) {
    state.set_goal_user_queue_pending(&conversation_id, pending);
}
fn response(mut c: Conversation) -> Value {
    crate::chat::commands::catalog::strip_transcripts_for_frontend(&mut c);
    serde_json::json!({"success":true,"goalState":c.goal_state,"conversation":c})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn completion_metadata_preserves_old_goals_and_round_trips_through_protocol() {
        let mut goal: GoalState = serde_json::from_value(serde_json::json!({
            "id":"g", "version":1, "objective":"Deliver a plan", "status":"completed",
            "created_at":10, "updated_at":20
        }))
        .unwrap();
        assert_eq!(goal.completed_at, None);
        assert_eq!(goal.completed_message_id, None);
        goal.completed_at = Some(18);
        goal.completed_message_id = Some("result".into());
        let saved: GoalState =
            serde_json::from_value(serde_json::to_value(&goal).unwrap()).unwrap();
        assert_eq!(saved, goal);
        let event = serde_json::to_value(crate::chat::protocol::ChatGoalStatePayload::from(&saved))
            .unwrap();
        assert_eq!(event["completedAt"], 18);
        assert_eq!(event["completedMessageId"], "result");
        assert_eq!(event["updatedAt"], 20);
    }
    #[test]
    fn fingerprint_normalizes_case_and_space() {
        assert_eq!(normalize_fingerprint(" Done!  NOW "), "done now");
    }
    #[test]
    fn repeated_or_empty_runs_accumulate_and_new_output_resets() {
        assert_eq!(next_no_progress_count(1, Some("same"), "same"), 2);
        assert_eq!(next_no_progress_count(2, Some("same"), ""), 3);
        assert_eq!(next_no_progress_count(2, Some("same"), "new evidence"), 0);
    }
    #[test]
    fn mutating_tool_detection_covers_builtin_writes() {
        assert!(is_mutating_tool("write"));
        assert!(is_mutating_tool("apply_patch"));
        assert!(!is_mutating_tool("read"));
    }
}
