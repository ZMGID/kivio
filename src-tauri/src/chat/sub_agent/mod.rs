//! OMP-style subagents (ADR-0007, docs/prd/omp-subagents-plan.md).
//! Behaviour follows oh-my-pi commit 5dec65e; see each module for its source.
//! Identity, run, job, call aggregate and delivery states remain separate.

use serde_json::Value;

use crate::agents::AgentDefinition;
use crate::mcp::ChatToolDefinition;

pub(crate) mod awaits;
pub mod commands;
pub mod delivery;
pub(crate) mod eval;
pub mod executor;
pub mod irc;
pub(crate) mod isolation;
pub mod jobs;
pub mod lifecycle;
pub mod names;
pub(crate) mod policies;
pub(crate) mod progress;
pub mod prompts;
pub mod registry;
pub(crate) mod seance;
pub(crate) mod services;
pub(crate) mod session;
pub(crate) mod spawn;
pub(crate) mod speculative;
pub(crate) mod structured;
pub mod task;
pub mod urls;
pub mod view;
pub mod wait;
pub(crate) mod workpool;

pub const TASK_TOOL_NAME: &str = "task";
pub const WAIT_TOOL_NAME: &str = "wait";
/// Owner id of a conversation's top-level agent (OMP `MAIN_AGENT_ID`).
pub const MAIN_AGENT_ID: &str = "Main";

pub fn is_sub_agent_tool_name(name: &str) -> bool {
    matches!(name, TASK_TOOL_NAME | WAIT_TOOL_NAME)
}

use crate::chat::agent::history::RuntimeMessage;
use crate::chat::agent::host::{InputPersistence, SessionInput};

pub(crate) fn session_input(message: RuntimeMessage) -> SessionInput {
    let persistence = if message
        .source
        .get("subagent_parent_persisted")
        .and_then(serde_json::Value::as_bool)
        == Some(true)
    {
        InputPersistence::AlreadyPersisted
    } else {
        InputPersistence::Append
    };
    SessionInput {
        message,
        persistence,
    }
}

pub async fn collect_for_main(
    app: &tauri::AppHandle,
    root: &str,
    run: &str,
    history: &[RuntimeMessage],
    admission: &mut kivio_agent_sdk::agent::InjectionAdmission,
) -> Result<(), String> {
    delivery::collect_for_main(app, root, run, history, admission).await?;
    irc::collect_for_main(app, root, run, history, admission).await
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
    pub(crate) speculative: speculative::Sessions,
    pub(crate) sessions: session::Sessions,
    pub irc: irc::Bus,
    pub(crate) awaits: awaits::AwaitGraph,
    pub(crate) waits: wait::WaitState,
    pub(crate) eval: eval::Kernels,
    pub(crate) pools: workpool::Pools,
    user_interrupted: std::sync::Mutex<std::collections::HashMap<String, Interruption>>,
}

#[derive(Default)]
struct Interruption {
    epoch: u64,
    interrupted: bool,
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
        self.sessions.forget(root);
        self.eval.forget_root(root);
        self.pools.forget_root(root);
        self.irc.forget(root);
        self.waits.forget(root);
        self.jobs.cancel_root(root);
        self.jobs.remove_root(root);
        self.registry.remove_root(root);
        self.set_user_interrupted(root, false);
    }

    /// OMP exception: after a user interrupt, finished results are not allowed to
    /// start a parent turn until the user prompts again.
    pub fn set_user_interrupted(&self, root: &str, interrupted: bool) {
        let mut states = self
            .user_interrupted
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let state = states.entry(root.to_string()).or_default();
        if interrupted {
            state.epoch = state.epoch.wrapping_add(1);
        }
        state.interrupted = interrupted;
    }

    pub fn user_interrupted(&self, root: &str) -> bool {
        self.user_interrupted
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(root)
            .is_some_and(|state| state.interrupted)
    }

    pub fn interruption_epoch(&self, root: &str) -> u64 {
        self.user_interrupted
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(root)
            .map_or(0, |state| state.epoch)
    }

    /// Lock order: interruption state before runtime run reservations.
    pub(crate) fn with_interruption_epoch<R>(
        &self,
        root: &str,
        action: impl FnOnce(u64) -> R,
    ) -> R {
        let states = self
            .user_interrupted
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        action(states.get(root).map_or(0, |state| state.epoch))
    }

    pub(crate) fn interrupt_user_turn<R>(&self, root: &str, action: impl FnOnce() -> R) -> R {
        let mut states = self
            .user_interrupted
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let state = states.entry(root.to_string()).or_default();
        state.epoch = state.epoch.wrapping_add(1);
        state.interrupted = true;
        action()
    }

    /// A Stop accepted while the new turn was being saved must win.
    pub fn resume_user_turn(&self, root: &str, epoch: u64) -> bool {
        let mut states = self
            .user_interrupted
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let state = states.entry(root.to_string()).or_default();
        if state.epoch != epoch {
            return false;
        }
        state.interrupted = false;
        true
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
    let task = (settings.chat_tools.task_max_recursion_depth != 0)
        .then(|| task::tool_definition(defs, settings));
    for tool in task
        .into_iter()
        .chain([wait::definition(), eval::definition()])
    {
        if !tools
            .iter()
            .any(|existing| existing.openai_tool_name() == tool.openai_tool_name())
        {
            tools.push(tool);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepted_turn_cannot_clear_a_newer_stop() {
        let manager = SubAgentManager::default();
        manager.set_user_interrupted("root", true);
        let epoch = manager.interruption_epoch("root");
        manager.set_user_interrupted("root", true);
        assert!(!manager.resume_user_turn("root", epoch));
        assert!(manager.user_interrupted("root"));
        assert!(manager.resume_user_turn("root", manager.interruption_epoch("root")));
        assert!(!manager.user_interrupted("root"));
    }

    #[test]
    fn zero_depth_hides_dispatch_but_preserves_wait() {
        let mut settings = crate::settings::Settings::default();
        settings.chat_tools.task_max_recursion_depth = 0;
        let mut tools = Vec::new();
        append_tool_definitions(
            &mut tools,
            &settings,
            &crate::agents::builtin_agent_definitions(),
        );
        assert!(!tools.iter().any(|tool| tool.name == TASK_TOOL_NAME));
        assert!(tools.iter().any(|tool| tool.name == WAIT_TOOL_NAME));
        settings.chat_tools.task_max_recursion_depth = -1;
        append_tool_definitions(
            &mut tools,
            &settings,
            &crate::agents::builtin_agent_definitions(),
        );
        assert!(tools.iter().any(|tool| tool.name == TASK_TOOL_NAME));
    }
}
