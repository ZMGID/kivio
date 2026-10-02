//! Chat tools: the model schedules follow-ups in the conversation it is
//! talking in. Tasks created here are always bound to that conversation, and
//! the tools only see/modify that conversation's tasks.

use chrono::{Local, NaiveDateTime, TimeZone};
use serde::Deserialize;
use serde_json::{json, Value};
use tauri::AppHandle;

use crate::mcp::native_registry::NativeToolFuture;
use crate::mcp::registry::NativeToolContext;
use crate::mcp::types::{ChatToolDefinition, McpToolCallResult};

use super::types::{ScheduleRule, ScheduledTask, ScheduledTaskInput, TaskSource, TaskTarget};
use super::{emit_changed, now_secs, service};

pub const CREATE_TOOL: &str = "schedule_create";
pub const LIST_TOOL: &str = "schedule_list";
pub const UPDATE_TOOL: &str = "schedule_update";
pub const DELETE_TOOL: &str = "schedule_delete";

/// Appends the tools unless this conversation is itself running a scheduled
/// prompt (a scheduled turn must not reschedule itself).
pub fn append_tools(app: &AppHandle, conversation_id: &str, tools: &mut Vec<ChatToolDefinition>) {
    if service(app).is_scheduled_conversation_busy(conversation_id) {
        return;
    }
    tools.extend([create_tool(), list_tool(), update_tool(), delete_tool()]);
}

fn schedule_schema() -> Value {
    json!({
        "type": "object",
        "description": "When to run. Times are the user's local time.",
        "properties": {
            "kind": { "type": "string", "enum": ["once", "interval", "daily", "weekly", "monthly", "yearly", "cron"] },
            "at": { "type": "string", "description": "once: local date-time `YYYY-MM-DD HH:MM`." },
            "minutes": { "type": "integer", "minimum": 1, "description": "interval: every N minutes." },
            "time": { "type": "string", "description": "daily/weekly/monthly/yearly: local time `HH:MM`." },
            "weekdays": { "type": "array", "items": { "type": "integer", "minimum": 0, "maximum": 6 }, "description": "weekly: 0=Sunday … 6=Saturday." },
            "days": { "type": "array", "items": { "type": "integer", "minimum": 1, "maximum": 31 }, "description": "monthly: days of month; months without that day are skipped." },
            "month": { "type": "integer", "minimum": 1, "maximum": 12, "description": "yearly: month." },
            "day": { "type": "integer", "minimum": 1, "maximum": 31, "description": "yearly: day of month." },
            "expr": { "type": "string", "description": "cron: five fields `minute hour day-of-month month day-of-week`." }
        },
        "required": ["kind"]
    })
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
        annotations: Some(json!({
            "readOnlyHint": read_only,
            "destructiveHint": name == DELETE_TOOL,
            "openWorldHint": false,
        })),
        output_schema: None,
    }
}

pub fn create_tool() -> ChatToolDefinition {
    tool(
        CREATE_TOOL,
        "Create a 定时任务 (scheduled task): a prompt sent automatically into THIS conversation later, as if the user sent it. \
This is the default for any timed or recurring request — 定时任务, 提醒, \"每天/每周/每月…帮我…\", \"X 分钟后…\", daily reports, periodic checks. \
Do NOT build an automation (automation_upsert) for these unless the user explicitly asks for an automation/workflow. \
For relative times (\"in 20 minutes\") pass `delay_minutes`; otherwise pass `schedule`. \
`prompt` must be self-contained: describe the work to do at that time, not \"remind me to schedule\". \
The result reports the next run time; tell it to the user.",
        json!({
            "type": "object",
            "properties": {
                "name": { "type": "string", "description": "Short task name." },
                "prompt": { "type": "string", "description": "The message that will be sent at run time." },
                "schedule": schedule_schema(),
                "delay_minutes": { "type": "integer", "minimum": 1, "description": "Run once, this many minutes from now." }
            },
            "required": ["prompt"]
        }),
        false,
    )
}

pub fn list_tool() -> ChatToolDefinition {
    tool(
        LIST_TOOL,
        "List the scheduled tasks bound to this conversation with their next run times.",
        json!({ "type": "object", "properties": {} }),
        true,
    )
}

pub fn update_tool() -> ChatToolDefinition {
    tool(
        UPDATE_TOOL,
        "Change a scheduled task of this conversation. Only the given fields change; `enabled: false` pauses it.",
        json!({
            "type": "object",
            "properties": {
                "id": { "type": "string" },
                "name": { "type": "string" },
                "prompt": { "type": "string" },
                "schedule": schedule_schema(),
                "delay_minutes": { "type": "integer", "minimum": 1 },
                "enabled": { "type": "boolean" }
            },
            "required": ["id"]
        }),
        false,
    )
}

pub fn delete_tool() -> ChatToolDefinition {
    tool(
        DELETE_TOOL,
        "Delete a scheduled task of this conversation and its run history.",
        json!({
            "type": "object",
            "properties": { "id": { "type": "string" } },
            "required": ["id"]
        }),
        false,
    )
}

#[derive(Deserialize, Default)]
struct ScheduleArgs {
    kind: String,
    at: Option<String>,
    minutes: Option<u32>,
    time: Option<String>,
    weekdays: Option<Vec<u8>>,
    expr: Option<String>,
    #[serde(default)]
    days: Option<Vec<u8>>,
    #[serde(default)]
    month: Option<u8>,
    #[serde(default)]
    day: Option<u8>,
}

#[derive(Deserialize)]
struct CreateArgs {
    #[serde(default)]
    name: String,
    prompt: String,
    schedule: Option<ScheduleArgs>,
    delay_minutes: Option<u32>,
}

#[derive(Deserialize)]
struct UpdateArgs {
    id: String,
    name: Option<String>,
    prompt: Option<String>,
    schedule: Option<ScheduleArgs>,
    delay_minutes: Option<u32>,
    enabled: Option<bool>,
}

#[derive(Deserialize)]
struct IdArgs {
    id: String,
}

pub fn handle_conversation_tool_call<'a>(
    app: &'a AppHandle,
    ctx: &'a NativeToolContext,
    tool_name: &'a str,
    arguments: Value,
) -> NativeToolFuture<'a> {
    Box::pin(async move { Ok(call(app, ctx, tool_name, arguments)) })
}

fn call(
    app: &AppHandle,
    ctx: &NativeToolContext,
    tool_name: &str,
    arguments: Value,
) -> McpToolCallResult {
    match dispatch(app, ctx, tool_name, arguments) {
        Ok(value) => result(value, false),
        Err(message) => result(json!({ "error": message }), true),
    }
}

fn dispatch(
    app: &AppHandle,
    ctx: &NativeToolContext,
    tool_name: &str,
    arguments: Value,
) -> Result<Value, String> {
    let conversation_id = ctx.conversation_id.as_str();
    let svc = service(app);
    if tool_name == LIST_TOOL {
        let tasks: Vec<Value> = bound_tasks(&svc.list(), conversation_id)
            .into_iter()
            .map(describe)
            .collect();
        return Ok(json!({ "tasks": tasks }));
    }
    if ctx.depth > 0 {
        return Err("子代理不能创建或修改定时任务".into());
    }
    if svc.is_scheduled_conversation_busy(conversation_id) {
        return Err("定时任务触发的这一轮对话里不能再创建或修改定时任务".into());
    }
    let now = now_secs();
    match tool_name {
        CREATE_TOOL => {
            let args: CreateArgs = parse(arguments)?;
            let schedule = schedule_from(args.schedule, args.delay_minutes, now)?
                .ok_or("需要提供 schedule 或 delay_minutes")?;
            let task = svc.save(
                ScheduledTaskInput {
                    id: None,
                    name: args.name,
                    prompt: args.prompt,
                    schedule,
                    target: TaskTarget::Conversation {
                        conversation_id: conversation_id.to_string(),
                    },
                    enabled: Some(true),
                },
                TaskSource::Chat,
                now,
            )?;
            emit_changed(app, &task.id, None);
            Ok(json!({ "created": describe(&task) }))
        }
        UPDATE_TOOL => {
            let args: UpdateArgs = parse(arguments)?;
            let task = owned_task(&svc.list(), conversation_id, &args.id)?;
            let schedule = schedule_from(args.schedule, args.delay_minutes, now)?;
            let saved = svc.save(
                ScheduledTaskInput {
                    id: Some(task.id.clone()),
                    name: args.name.unwrap_or(task.name.clone()),
                    prompt: args.prompt.unwrap_or(task.prompt.clone()),
                    schedule: schedule.unwrap_or(task.schedule.clone()),
                    target: TaskTarget::Conversation {
                        conversation_id: task.conversation_id.clone(),
                    },
                    enabled: Some(args.enabled.unwrap_or(task.enabled)),
                },
                task.source,
                now,
            )?;
            emit_changed(app, &saved.id, None);
            Ok(json!({ "updated": describe(&saved) }))
        }
        DELETE_TOOL => {
            let args: IdArgs = parse(arguments)?;
            let task = owned_task(&svc.list(), conversation_id, &args.id)?;
            svc.delete(&task.id)?;
            emit_changed(app, &task.id, None);
            Ok(json!({ "deleted": task.id }))
        }
        other => Err(format!("unknown scheduled task tool: {other}")),
    }
}

fn parse<T: for<'de> Deserialize<'de>>(arguments: Value) -> Result<T, String> {
    serde_json::from_value(arguments).map_err(|err| format!("参数无效：{err}"))
}

fn bound_tasks<'a>(tasks: &'a [ScheduledTask], conversation_id: &str) -> Vec<&'a ScheduledTask> {
    tasks
        .iter()
        .filter(|task| task.conversation_id == conversation_id)
        .collect()
}

fn owned_task(
    tasks: &[ScheduledTask],
    conversation_id: &str,
    id: &str,
) -> Result<ScheduledTask, String> {
    bound_tasks(tasks, conversation_id)
        .into_iter()
        .find(|task| task.id == id)
        .cloned()
        .ok_or_else(|| "这个对话下没有该定时任务".to_string())
}

fn schedule_from(
    schedule: Option<ScheduleArgs>,
    delay_minutes: Option<u32>,
    now: i64,
) -> Result<Option<ScheduleRule>, String> {
    match (schedule, delay_minutes) {
        (Some(_), Some(_)) => Err("schedule 与 delay_minutes 只能二选一".into()),
        (None, Some(minutes)) if minutes >= 1 => Ok(Some(ScheduleRule::Once {
            at: now + i64::from(minutes) * 60,
        })),
        (None, Some(_)) => Err("delay_minutes 至少为 1".into()),
        (None, None) => Ok(None),
        (Some(args), None) => rule_from_args(args).map(Some),
    }
}

fn rule_from_args(args: ScheduleArgs) -> Result<ScheduleRule, String> {
    match args.kind.as_str() {
        "once" => {
            let text = args.at.ok_or("once 需要 at")?;
            let naive = [
                "%Y-%m-%d %H:%M",
                "%Y-%m-%dT%H:%M",
                "%Y-%m-%d %H:%M:%S",
                "%Y-%m-%dT%H:%M:%S",
            ]
            .iter()
            .find_map(|format| NaiveDateTime::parse_from_str(text.trim(), format).ok())
            .ok_or("at 须为本地时间 YYYY-MM-DD HH:MM")?;
            let at = Local
                .from_local_datetime(&naive)
                .earliest()
                .ok_or("该本地时间不存在（夏令时切换）")?;
            Ok(ScheduleRule::Once { at: at.timestamp() })
        }
        "interval" => Ok(ScheduleRule::Interval {
            minutes: args.minutes.ok_or("interval 需要 minutes")?,
            anchor_at: None,
        }),
        "daily" => {
            let (hour, minute) = parse_time(args.time.as_deref())?;
            Ok(ScheduleRule::Daily { hour, minute })
        }
        "weekly" => {
            let (hour, minute) = parse_time(args.time.as_deref())?;
            Ok(ScheduleRule::Weekly {
                weekdays: args.weekdays.ok_or("weekly 需要 weekdays")?,
                hour,
                minute,
            })
        }
        "monthly" => {
            let (hour, minute) = parse_time(args.time.as_deref())?;
            Ok(ScheduleRule::Monthly {
                days: args.days.ok_or("monthly 需要 days")?,
                hour,
                minute,
            })
        }
        "yearly" => {
            let (hour, minute) = parse_time(args.time.as_deref())?;
            Ok(ScheduleRule::Yearly {
                month: args.month.ok_or("yearly 需要 month")?,
                day: args.day.ok_or("yearly 需要 day")?,
                hour,
                minute,
            })
        }
        "cron" => Ok(ScheduleRule::Cron {
            expr: args.expr.ok_or("cron 需要 expr")?,
        }),
        other => Err(format!("未知的 schedule.kind：{other}")),
    }
}

fn parse_time(time: Option<&str>) -> Result<(u8, u8), String> {
    let time = time.ok_or("需要 time（HH:MM）")?;
    let (hour, minute) = time.trim().split_once(':').ok_or("time 须为 HH:MM")?;
    let hour = hour.parse::<u8>().map_err(|_| "time 须为 HH:MM")?;
    let minute = minute.parse::<u8>().map_err(|_| "time 须为 HH:MM")?;
    Ok((hour, minute))
}

fn format_local(timestamp: Option<i64>) -> Value {
    timestamp
        .and_then(|ts| Local.timestamp_opt(ts, 0).single())
        .map(|time| Value::String(time.format("%Y-%m-%d %H:%M (%a)").to_string()))
        .unwrap_or(Value::Null)
}

fn describe(task: &ScheduledTask) -> Value {
    json!({
        "id": task.id,
        "name": task.name,
        "prompt": task.prompt,
        "schedule": task.schedule,
        "enabled": task.enabled,
        "status": task.status,
        "nextRunAt": format_local(task.next_run_at),
        "lastRunAt": format_local(task.last_run_at),
        "runCount": task.run_count,
        "lastError": task.last_error,
    })
}

fn result(value: Value, is_error: bool) -> McpToolCallResult {
    McpToolCallResult {
        content: serde_json::to_string_pretty(&value).unwrap_or_default(),
        is_error,
        raw: value,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delay_minutes_becomes_one_shot_from_now() {
        assert_eq!(
            schedule_from(None, Some(20), 1_000).unwrap(),
            Some(ScheduleRule::Once { at: 2_200 })
        );
        assert!(schedule_from(
            Some(ScheduleArgs {
                kind: "daily".into(),
                at: None,
                minutes: None,
                time: Some("9:00".into()),
                weekdays: None,
                expr: None,
                ..Default::default()
            }),
            Some(5),
            0
        )
        .is_err());
    }

    #[test]
    fn weekly_args_map_to_rule() {
        let rule = rule_from_args(ScheduleArgs {
            kind: "weekly".into(),
            at: None,
            minutes: None,
            time: Some("08:30".into()),
            weekdays: Some(vec![1, 5]),
            expr: None,
            ..Default::default()
        })
        .unwrap();
        assert_eq!(
            rule,
            ScheduleRule::Weekly {
                weekdays: vec![1, 5],
                hour: 8,
                minute: 30
            }
        );
    }
}
