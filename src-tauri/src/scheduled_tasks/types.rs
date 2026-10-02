use serde::{Deserialize, Serialize};

/// When a task fires. Times are interpreted in the machine's local time zone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ScheduleRule {
    /// Fire once at a unix timestamp (seconds).
    Once {
        at: i64,
    },
    /// Every `minutes`, counted from `anchor_at` so late dispatches never drift.
    #[serde(rename_all = "camelCase")]
    Interval {
        minutes: u32,
        #[serde(default)]
        anchor_at: Option<i64>,
    },
    Daily {
        hour: u8,
        minute: u8,
    },
    /// `weekdays`: 0 = Sunday … 6 = Saturday.
    Weekly {
        weekdays: Vec<u8>,
        hour: u8,
        minute: u8,
    },
    /// `days`: day-of-month 1…31; months without that day are skipped.
    Monthly {
        days: Vec<u8>,
        hour: u8,
        minute: u8,
    },
    /// Feb 29 fires only in leap years.
    Yearly {
        month: u8,
        day: u8,
        hour: u8,
        minute: u8,
    },
    /// Five-field cron expression.
    Cron {
        expr: String,
    },
}

/// Where a task's prompts go, as requested by the editor. A saved task always
/// stores one concrete conversation: `NewConversation` is resolved at save
/// time by creating that conversation (named after the task) and binding it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum TaskTarget {
    /// Create the conversation now. An explicit provider+model pins the
    /// built-in agent runtime; otherwise the user's default runtime/model applies.
    #[serde(rename_all = "camelCase")]
    NewConversation {
        #[serde(default)]
        provider_id: Option<String>,
        #[serde(default)]
        model: Option<String>,
        #[serde(default)]
        project_id: Option<String>,
        /// 思考等级 (`off|low|medium|high`); None follows global.
        #[serde(default)]
        thinking_level: Option<String>,
    },
    /// Bind an existing conversation; it keeps its own runtime/model.
    #[serde(rename_all = "camelCase")]
    Conversation { conversation_id: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TaskStatus {
    Active,
    /// A one-shot task that already fired. It has no next run.
    Completed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TaskSource {
    User,
    /// Created by the model through a chat tool; bound to that conversation.
    Chat,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduledTask {
    pub id: String,
    pub name: String,
    pub prompt: String,
    pub schedule: ScheduleRule,
    /// Every run is sent into this conversation.
    pub conversation_id: String,
    pub enabled: bool,
    pub status: TaskStatus,
    pub next_run_at: Option<i64>,
    pub last_run_at: Option<i64>,
    pub run_count: u32,
    pub last_error: Option<String>,
    pub source: TaskSource,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Create (no id) or full update (id) request.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduledTaskInput {
    #[serde(default)]
    pub id: Option<String>,
    pub name: String,
    pub prompt: String,
    pub schedule: ScheduleRule,
    pub target: TaskTarget,
    #[serde(default)]
    pub enabled: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RunTrigger {
    Schedule,
    Manual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RunStatus {
    /// Waiting for the target conversation to finish its current reply.
    Queued,
    Running,
    Succeeded,
    Failed,
    /// Not sent (missed while asleep/closed, or a previous run still queued).
    Skipped,
    /// The app exited before the run finished.
    Interrupted,
}

impl RunStatus {
    pub fn is_terminal(self) -> bool {
        !matches!(self, RunStatus::Queued | RunStatus::Running)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskRun {
    pub id: String,
    pub task_id: String,
    pub trigger: RunTrigger,
    pub scheduled_at: Option<i64>,
    pub status: RunStatus,
    pub conversation_id: Option<String>,
    pub error: Option<String>,
    pub created_at: i64,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
}
