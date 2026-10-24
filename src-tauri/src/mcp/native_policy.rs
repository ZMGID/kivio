//! Shared builtin tool exposure, approval and scheduling policy.
//! Declaration order is stable and matches the host execution registry.
use crate::settings::ChatNativeToolsConfig;

pub type NativeToolEnabledFn = fn(&ChatNativeToolsConfig, bool, bool) -> bool;

pub struct NativeToolPolicy {
    pub name: &'static str,
    pub enabled: NativeToolEnabledFn,
    pub parallel_safe: bool,
    pub bypasses_approval: bool,
    pub read_only: bool,
    pub requires_session_consent: bool,
}

pub const WEB_SEARCH: NativeToolPolicy = NativeToolPolicy {
        name: "web_search",
        enabled: |native, web_search_configured, _| native.web_search && web_search_configured,
        parallel_safe: true,
        bypasses_approval: false,
        read_only: true,
        requires_session_consent: false,
};

pub const WEB_FETCH: NativeToolPolicy = NativeToolPolicy {
        name: "web_fetch",
        enabled: |native, _, _| native.web_fetch,
        parallel_safe: true,
        bypasses_approval: false,
        read_only: true,
        requires_session_consent: false,
};

pub const KNOWLEDGE_SEARCH: NativeToolPolicy = NativeToolPolicy {
        name: "knowledge_search",
        enabled: |native, _, _| native.knowledge_search,
        parallel_safe: true,
        bypasses_approval: false,
        read_only: true,
        requires_session_consent: false,
};

pub const AUTOMATION_LIST: NativeToolPolicy = NativeToolPolicy {
        name: "automation_list",
        enabled: |native, _, _| native.automation,
        parallel_safe: true,
        bypasses_approval: false,
        read_only: true,
        requires_session_consent: false,
};

pub const AUTOMATION_GET: NativeToolPolicy = NativeToolPolicy {
        name: "automation_get",
        enabled: |native, _, _| native.automation,
        parallel_safe: true,
        bypasses_approval: false,
        read_only: true,
        requires_session_consent: false,
};

pub const AUTOMATION_UPSERT: NativeToolPolicy = NativeToolPolicy {
        name: "automation_upsert",
        enabled: |native, _, _| native.automation,
        parallel_safe: false,
        bypasses_approval: false,
        read_only: false,
        requires_session_consent: false,
};

pub const AUTOMATION_SET_ENABLED: NativeToolPolicy = NativeToolPolicy {
        name: "automation_set_enabled",
        enabled: |native, _, _| native.automation,
        parallel_safe: false,
        bypasses_approval: false,
        read_only: false,
        requires_session_consent: false,
};

pub const AUTOMATION_RUN: NativeToolPolicy = NativeToolPolicy {
        name: "automation_run",
        enabled: |native, _, _| native.automation,
        parallel_safe: false,
        bypasses_approval: false,
        read_only: false,
        requires_session_consent: false,
};

pub const AUTOMATION_RUNS: NativeToolPolicy = NativeToolPolicy {
        name: "automation_runs",
        enabled: |native, _, _| native.automation,
        parallel_safe: true,
        bypasses_approval: false,
        read_only: true,
        requires_session_consent: false,
};

pub const AUTOMATION_DELETE: NativeToolPolicy = NativeToolPolicy {
        name: "automation_delete",
        enabled: |native, _, _| native.automation,
        parallel_safe: false,
        bypasses_approval: false,
        read_only: false,
        requires_session_consent: false,
};

pub const ADVISOR: NativeToolPolicy = NativeToolPolicy {
        name: "advisor",
        enabled: |_, _, _| false,
        parallel_safe: true,
        bypasses_approval: true,
        read_only: true,
        requires_session_consent: false,
};

pub const READ: NativeToolPolicy = NativeToolPolicy {
        name: "read",
        enabled: |native, _, _| native.read_file,
        parallel_safe: true,
        bypasses_approval: false,
        read_only: true,
        requires_session_consent: true,
};

pub const LS: NativeToolPolicy = NativeToolPolicy {
        name: "ls",
        enabled: |_, _, _| false,
        parallel_safe: true,
        bypasses_approval: false,
        read_only: true,
        requires_session_consent: true,
};

pub const GREP: NativeToolPolicy = NativeToolPolicy {
        name: "grep",
        enabled: |native, _, _| native.read_file,
        parallel_safe: true,
        bypasses_approval: false,
        read_only: true,
        requires_session_consent: true,
};

pub const GLOB: NativeToolPolicy = NativeToolPolicy {
        name: "glob",
        enabled: |native, _, _| native.read_file,
        parallel_safe: true,
        bypasses_approval: false,
        read_only: true,
        requires_session_consent: true,
};

pub const WRITE: NativeToolPolicy = NativeToolPolicy {
        name: "write",
        enabled: |native, _, _| native.write_file,
        parallel_safe: false,
        bypasses_approval: false,
        read_only: false,
        requires_session_consent: true,
};

pub const EDIT: NativeToolPolicy = NativeToolPolicy {
        name: "edit",
        enabled: |native, _, _| native.edit_file,
        parallel_safe: false,
        bypasses_approval: false,
        read_only: false,
        requires_session_consent: true,
};

pub const BASH: NativeToolPolicy = NativeToolPolicy {
        name: "bash",
        enabled: |native, _, _| native.run_command,
        parallel_safe: false,
        bypasses_approval: false,
        read_only: false,
        requires_session_consent: true,
};

pub const BASH_OUTPUT: NativeToolPolicy = NativeToolPolicy {
        name: "bash_output",
        enabled: |native, _, _| native.run_command,
        parallel_safe: true,
        bypasses_approval: false,
        read_only: true,
        requires_session_consent: true,
};

pub const KILL_BACKGROUND: NativeToolPolicy = NativeToolPolicy {
        name: "kill_background",
        enabled: |native, _, _| native.run_command,
        parallel_safe: false,
        bypasses_approval: false,
        read_only: false,
        requires_session_consent: true,
};

pub const SAVE_ASSISTANT: NativeToolPolicy = NativeToolPolicy {
        name: "save_assistant",
        enabled: |_, _, _| false,
        parallel_safe: false,
        bypasses_approval: false,
        read_only: false,
        requires_session_consent: false,
};

pub const PRESENT_ARTIFACTS: NativeToolPolicy = NativeToolPolicy {
        name: "present_artifacts",
        enabled: |_, _, _| true,
        parallel_safe: false,
        bypasses_approval: true,
        read_only: true,
        requires_session_consent: false,
};

pub const KIVIO_INSPECT: NativeToolPolicy = NativeToolPolicy {
        name: "kivio_inspect",
        enabled: |native, _, _| native.read_file,
        parallel_safe: false,
        bypasses_approval: false,
        read_only: true,
        requires_session_consent: true,
};

pub const KIVIO_CONFIGURE: NativeToolPolicy = NativeToolPolicy {
        name: "kivio_configure",
        enabled: |native, _, _| native.run_command,
        parallel_safe: false,
        bypasses_approval: false,
        read_only: false,
        requires_session_consent: true,
};

pub const MEMORY_READ: NativeToolPolicy = NativeToolPolicy {
        name: "memory_read",
        enabled: |_, _, memory_enabled| memory_enabled,
        parallel_safe: false,
        bypasses_approval: true,
        read_only: true,
        requires_session_consent: false,
};

pub const MEMORY_MODIFY: NativeToolPolicy = NativeToolPolicy {
        name: "memory_modify",
        enabled: |_, _, memory_enabled| memory_enabled,
        parallel_safe: false,
        bypasses_approval: true,
        read_only: false,
        requires_session_consent: false,
};

pub const MEMORY_SEARCH: NativeToolPolicy = NativeToolPolicy {
        name: "memory_search",
        enabled: |_, _, memory_enabled| memory_enabled,
        parallel_safe: false,
        bypasses_approval: true,
        read_only: true,
        requires_session_consent: false,
};

pub const SAVE_PLAN: NativeToolPolicy = NativeToolPolicy {
        name: "save_plan",
        enabled: |_, _, _| false,
        parallel_safe: false,
        bypasses_approval: true,
        read_only: false,
        requires_session_consent: false,
};

pub const TODO_WRITE_TOOL_NAME: NativeToolPolicy = NativeToolPolicy {
        name: "todo_write",
        enabled: |_, _, _| false,
        parallel_safe: false,
        bypasses_approval: true,
        read_only: false,
        requires_session_consent: false,
};

pub const GET_GOAL_TOOL: NativeToolPolicy = NativeToolPolicy {
        name: "get_goal",
        enabled: |_, _, _| false,
        parallel_safe: false,
        bypasses_approval: true,
        read_only: true,
        requires_session_consent: false,
};

pub const INIT_GOAL_CRITERIA_TOOL: NativeToolPolicy = NativeToolPolicy {
        name: "initialize_goal_criteria",
        enabled: |_, _, _| false,
        parallel_safe: false,
        bypasses_approval: true,
        read_only: false,
        requires_session_consent: false,
};

pub const REPORT_GOAL_PROGRESS_TOOL: NativeToolPolicy = NativeToolPolicy {
        name: "report_goal_progress",
        enabled: |_, _, _| false,
        parallel_safe: false,
        bypasses_approval: true,
        read_only: false,
        requires_session_consent: false,
};

pub const COMPLETE_GOAL_TOOL: NativeToolPolicy = NativeToolPolicy {
        name: "goal_complete",
        enabled: |_, _, _| false,
        parallel_safe: false,
        bypasses_approval: true,
        read_only: false,
        requires_session_consent: false,
};

pub const BLOCK_GOAL_TOOL: NativeToolPolicy = NativeToolPolicy {
        name: "goal_blocked",
        enabled: |_, _, _| false,
        parallel_safe: false,
        bypasses_approval: true,
        read_only: false,
        requires_session_consent: false,
};

pub const WAIT_GOAL_TOOL: NativeToolPolicy = NativeToolPolicy {
        name: "goal_wait",
        enabled: |_, _, _| false,
        parallel_safe: false,
        bypasses_approval: true,
        read_only: false,
        requires_session_consent: false,
};

pub const LIST_TOOL: NativeToolPolicy = NativeToolPolicy {
        name: "schedule_list",
        enabled: |_, _, _| false,
        parallel_safe: false,
        bypasses_approval: true,
        read_only: true,
        requires_session_consent: false,
};

pub const CREATE_TOOL: NativeToolPolicy = NativeToolPolicy {
        name: "schedule_create",
        enabled: |_, _, _| false,
        parallel_safe: false,
        bypasses_approval: false,
        read_only: false,
        requires_session_consent: false,
};

pub const UPDATE_TOOL: NativeToolPolicy = NativeToolPolicy {
        name: "schedule_update",
        enabled: |_, _, _| false,
        parallel_safe: false,
        bypasses_approval: false,
        read_only: false,
        requires_session_consent: false,
};

pub const DELETE_TOOL: NativeToolPolicy = NativeToolPolicy {
        name: "schedule_delete",
        enabled: |_, _, _| false,
        parallel_safe: false,
        bypasses_approval: false,
        read_only: false,
        requires_session_consent: false,
};

pub const ASK_USER_TOOL_NAME: NativeToolPolicy = NativeToolPolicy {
        name: "ask_user",
        enabled: |_, _, _| false,
        parallel_safe: false,
        bypasses_approval: true,
        read_only: false,
        requires_session_consent: false,
};

pub const AGENT_TOOL_NAME: NativeToolPolicy = NativeToolPolicy {
        name: "agent",
        enabled: |_, _, _| false,
        parallel_safe: true,
        bypasses_approval: true,
        read_only: false,
        requires_session_consent: false,
};

pub const AGENT_CONTROL: NativeToolPolicy = NativeToolPolicy {
        name: "agent_control",
        enabled: |_, _, _| false,
        parallel_safe: false,
        bypasses_approval: true,
        read_only: false,
        requires_session_consent: false,
};

pub static NATIVE_POLICIES: &[NativeToolPolicy] = &[
    WEB_SEARCH,
    WEB_FETCH,
    KNOWLEDGE_SEARCH,
    AUTOMATION_LIST,
    AUTOMATION_GET,
    AUTOMATION_UPSERT,
    AUTOMATION_SET_ENABLED,
    AUTOMATION_RUN,
    AUTOMATION_RUNS,
    AUTOMATION_DELETE,
    ADVISOR,
    READ,
    LS,
    GREP,
    GLOB,
    WRITE,
    EDIT,
    BASH,
    BASH_OUTPUT,
    KILL_BACKGROUND,
    SAVE_ASSISTANT,
    PRESENT_ARTIFACTS,
    KIVIO_INSPECT,
    KIVIO_CONFIGURE,
    MEMORY_READ,
    MEMORY_MODIFY,
    MEMORY_SEARCH,
    SAVE_PLAN,
    TODO_WRITE_TOOL_NAME,
    GET_GOAL_TOOL,
    INIT_GOAL_CRITERIA_TOOL,
    REPORT_GOAL_PROGRESS_TOOL,
    COMPLETE_GOAL_TOOL,
    BLOCK_GOAL_TOOL,
    WAIT_GOAL_TOOL,
    LIST_TOOL,
    CREATE_TOOL,
    UPDATE_TOOL,
    DELETE_TOOL,
    ASK_USER_TOOL_NAME,
    AGENT_TOOL_NAME,
    AGENT_CONTROL,
];
pub fn find_entry(name: &str) -> Option<&'static NativeToolPolicy> {
    NATIVE_POLICIES.iter().find(|entry| entry.name == name)
}
pub fn native_tool_requires_session_consent(name: &str) -> bool {
    find_entry(name).is_some_and(|entry| entry.requires_session_consent)
}

pub fn is_sub_agent_tool_name(name: &str) -> bool {
    name == AGENT_TOOL_NAME.name || name == AGENT_CONTROL.name
}
