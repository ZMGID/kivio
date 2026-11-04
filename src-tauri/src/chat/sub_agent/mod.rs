//! OMP-style subagents (ADR-0007, docs/prd/omp-subagents-plan.md).
//! Behaviour follows oh-my-pi commit 5dec65e; see each module for its source.
//! Three separate state machines: agent registry status, job status, run status.

use serde_json::Value;

use crate::agents::AgentDefinition;
use crate::mcp::ChatToolDefinition;

pub mod delivery;
pub mod executor;
pub mod jobs;
pub mod names;
pub mod prompts;
pub mod registry;
pub mod task;
pub mod urls;
pub mod wait;

pub const TASK_TOOL_NAME: &str = "task";
pub const WAIT_TOOL_NAME: &str = "wait";
/// Owner id of a conversation's top-level agent (OMP `MAIN_AGENT_ID`).
pub const MAIN_AGENT_ID: &str = "Main";

pub fn is_sub_agent_tool_name(name: &str) -> bool {
    matches!(name, TASK_TOOL_NAME | WAIT_TOOL_NAME)
}

/// Context handed to `task` / `wait` handlers, dispatched before workspace resolution.
pub struct SubAgentCallCtx<'a> {
    pub app: &'a tauri::AppHandle,
    pub state: &'a crate::state::AppState,
    pub native_ctx: &'a crate::mcp::registry::NativeToolContext<'a>,
    pub arguments: &'a Value,
}

/// Process-owned subagent state: agent registry, owner-scoped jobs, spawn admission.
#[derive(Default)]
pub struct SubAgentManager {
    pub registry: registry::AgentRegistry,
    pub jobs: jobs::JobManager,
    pub(crate) admission: task::Admission,
    user_interrupted: std::sync::Mutex<std::collections::HashSet<String>>,
}

impl SubAgentManager {
    /// Parent conversation stopped: cancel every job it owns; agents stay registered.
    pub fn stop_conversation(&self, root: &str) {
        self.jobs.cancel_root(root);
    }

    /// Conversation deleted: drop all of its agents, jobs and admission state.
    pub fn forget_conversation(&self, root: &str) {
        // Admission first: `remove_root` permanently tombstones the root under the
        // allocation guard, so no in-flight or later `task` dispatch can re-register.
        self.admission.remove_root(root);
        self.jobs.cancel_root(root);
        self.jobs.remove_root(root);
        self.registry.remove_root(root);
        self.set_user_interrupted(root, false);
    }

    /// Child conversation ids (usage records) of every agent under `root`.
    pub fn child_conversation_ids(&self, root: &str) -> Vec<String> {
        self.registry
            .list(root)
            .into_iter()
            .map(|agent| agent.child_conversation_id)
            .collect()
    }

    /// OMP exception: after a user interrupt, finished results are not allowed to
    /// start a parent turn until the user prompts again.
    pub fn set_user_interrupted(&self, root: &str, interrupted: bool) {
        let mut set = self
            .user_interrupted
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if interrupted {
            set.insert(root.to_string());
        } else {
            set.remove(root);
        }
    }

    pub fn user_interrupted(&self, root: &str) -> bool {
        self.user_interrupted
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains(root)
    }

    /// Agent id that owns `run_id` (a child run), or `MAIN_AGENT_ID` for any other run.
    pub fn owner_for_run(&self, run_id: &str) -> String {
        self.registry
            .find_by_run(run_id)
            .map(|(_, id)| id)
            .unwrap_or_else(|| MAIN_AGENT_ID.to_string())
    }
}

/// Model-facing `task` and `wait` for a top-level run (children get theirs from the executor).
pub fn append_tool_definitions(
    tools: &mut Vec<ChatToolDefinition>,
    settings: &crate::settings::Settings,
    defs: &[AgentDefinition],
) {
    for tool in [task::tool_definition(defs, settings), wait::definition()] {
        if !tools
            .iter()
            .any(|existing| existing.openai_tool_name() == tool.openai_tool_name())
        {
            tools.push(tool);
        }
    }
}
