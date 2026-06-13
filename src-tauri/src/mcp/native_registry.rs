//! Static registry for Kivio's builtin (native-source) tools.
//!
//! This table is the single source of truth that replaces the previously
//! drifting hardcoded lists:
//! - exposure if-chain in `types.rs::list_native_builtin_tool_defs`
//! - dispatch match in `registry.rs::call_native_tool`
//! - `BUILTIN_NAMES` in `chat/agent/prepare.rs::disabled_builtin_tool_feedback`
//! - approval bypass list in `chat/agent/prepare.rs::builtin_tool_bypasses_approval`
//! - parallel whitelist in `chat/agent/rounds.rs::tool_call_parallel_eligible`
//! - native read-only arm in `types.rs::ChatToolDefinition::is_read_only_tool`
//!
//! Contract notes (see `.trellis/spec/backend/agent-runtime.md` and
//! `.trellis/spec/backend/file-tools.md`):
//! - The `parallel_safe` set is intentionally narrow: web_search/web_fetch/
//!   read_file plus the read-side project tools (list_dir/search_files/
//!   glob_files/stat_path), and only when approval-free. Do not widen or
//!   narrow it here without a spec change. memory_read is read-only and
//!   approval-free but deliberately NOT parallel-safe. `agent` is also
//!   parallel-safe (multi-agent fan-out): each spawn runs isolated and is
//!   Semaphore(3)-capped.
//! - Table order is the model-facing tool list order; keep it stable.

use std::future::Future;
use std::pin::Pin;

use serde_json::Value;
use tauri::AppHandle;

use crate::native_tools::{FileMutationResult, NativeToolWorkspace};
use crate::settings::{ChatNativeToolsConfig, Settings};
use crate::state::AppState;

use super::registry::NativeToolContext;
use super::types::{
    native_copy_path_tool, native_create_dir_tool, native_delete_path_tool, native_edit_file_tool,
    native_glob_files_tool, native_list_dir_tool, native_memory_modify_tool,
    native_memory_read_tool, native_move_path_tool, native_read_file_tool, native_run_command_tool,
    native_run_python_tool, native_search_files_tool, native_stat_path_tool, native_web_fetch_tool,
    native_web_search_tool, native_write_file_tool, ChatToolDefinition, McpToolCallResult,
};

/// Gate signature mirrors `list_native_builtin_tool_defs(native,
/// web_search_configured, memory_enabled)` so exposure stays bit-identical.
pub type NativeToolEnabledFn = fn(&ChatNativeToolsConfig, bool, bool) -> bool;

pub type NativeToolFuture<'a> =
    Pin<Box<dyn Future<Output = Result<McpToolCallResult, String>> + Send + 'a>>;

/// Full-context async call. `workspace` is already resolved by
/// `call_native_tool` before dispatch, matching the legacy behavior where
/// every native tool resolved the workspace at entry.
pub struct NativeCallCtx<'a> {
    pub app: &'a AppHandle,
    pub state: &'a AppState,
    pub settings: &'a Settings,
    pub workspace: &'a NativeToolWorkspace,
    pub arguments: &'a Value,
    pub native_ctx: Option<&'a NativeToolContext>,
}

/// How `call_native_tool` dispatches a registry entry. The blocking-vs-sync
/// split is kept explicit here so "which tools run on the blocking thread
/// pool" stays auditable in one place (see `run_blocking_file_mutation`).
pub enum NativeToolCall {
    /// Synchronous, workspace-scoped, plain-text result.
    SyncText(fn(&NativeToolWorkspace, &Value) -> Result<String, String>),
    /// Synchronous, workspace-scoped, custom tool result.
    SyncResult(fn(&NativeToolWorkspace, &Value) -> Result<McpToolCallResult, String>),
    /// spawn_blocking, plain-text result (path mutations with lock waits).
    BlockingText(fn(&NativeToolWorkspace, &Value) -> Result<String, String>),
    /// spawn_blocking, structured `FileMutationResult` (write_file/edit_file).
    BlockingMutation(fn(&NativeToolWorkspace, &Value) -> Result<FileMutationResult, String>),
    /// Full-context async call (web/memory/shell/python).
    Async(for<'a> fn(NativeCallCtx<'a>) -> NativeToolFuture<'a>),
    /// Conversation-scoped call (todo tools): runs before workspace
    /// resolution because it only needs the conversation id, matching the
    /// legacy `RegistryToolExecutor` special case which never resolved a
    /// workspace for todo tools.
    Conversation(fn(&AppHandle, &str, &str, Value) -> Result<McpToolCallResult, String>),
    /// Host-mediated tool (ask_user): intercepted in
    /// `chat/agent/execute.rs::execute_ask_user_call` and must never reach
    /// the registry dispatcher.
    HostMediated,
    /// Sub-agent management tools (agent / check_agent_result /
    /// list_agent_tasks): dispatched before workspace resolution (they manage
    /// agents, not files) with the parent run context from `NativeToolContext`
    /// (depth, run_id, generation, parent conversation/tool-call id).
    SubAgent(for<'a> fn(crate::chat::sub_agent::SubAgentCallCtx<'a>) -> NativeToolFuture<'a>),
}

pub struct NativeToolEntry {
    pub name: &'static str,
    pub def: fn() -> ChatToolDefinition,
    /// Whether the tool is exposed in the model tool list for the given
    /// settings. todo/ask_user return false here because they are appended
    /// separately in `chat/commands.rs` (`append_agent_todo_tools` /
    /// `append_agent_ask_user_tools`), not via
    /// `list_native_builtin_tool_defs`.
    pub enabled: NativeToolEnabledFn,
    pub parallel_safe: bool,
    pub bypasses_approval: bool,
    pub read_only: bool,
    pub call: NativeToolCall,
}

/// Declaration order is the model-facing exposure order (the legacy push
/// order of `list_native_builtin_tool_defs`), followed by the
/// conversation-level tools that are appended elsewhere.
pub static NATIVE_TOOLS: &[NativeToolEntry] = &[
    NativeToolEntry {
        name: "web_search",
        def: native_web_search_tool,
        enabled: |native, web_search_configured, _| native.web_search && web_search_configured,
        parallel_safe: true,
        bypasses_approval: false,
        read_only: true,
        call: NativeToolCall::Async(call_web_search),
    },
    NativeToolEntry {
        name: "web_fetch",
        def: native_web_fetch_tool,
        enabled: |native, _, _| native.web_fetch,
        parallel_safe: true,
        bypasses_approval: false,
        read_only: true,
        call: NativeToolCall::Async(call_web_fetch),
    },
    NativeToolEntry {
        name: "read_file",
        def: native_read_file_tool,
        enabled: |native, _, _| native.read_file,
        parallel_safe: true,
        bypasses_approval: false,
        read_only: true,
        call: NativeToolCall::SyncResult(call_read_file),
    },
    NativeToolEntry {
        name: "list_dir",
        def: native_list_dir_tool,
        enabled: |native, _, _| native.read_file,
        parallel_safe: true,
        bypasses_approval: false,
        read_only: true,
        call: NativeToolCall::SyncText(crate::native_tools::list_dir),
    },
    NativeToolEntry {
        name: "search_files",
        def: native_search_files_tool,
        enabled: |native, _, _| native.read_file,
        parallel_safe: true,
        bypasses_approval: false,
        read_only: true,
        call: NativeToolCall::SyncText(crate::native_tools::search_files),
    },
    NativeToolEntry {
        name: "glob_files",
        def: native_glob_files_tool,
        enabled: |native, _, _| native.read_file,
        parallel_safe: true,
        bypasses_approval: false,
        read_only: true,
        call: NativeToolCall::SyncText(crate::native_tools::glob_files),
    },
    NativeToolEntry {
        name: "stat_path",
        def: native_stat_path_tool,
        enabled: |native, _, _| native.read_file,
        parallel_safe: true,
        bypasses_approval: false,
        read_only: true,
        call: NativeToolCall::SyncText(crate::native_tools::stat_path),
    },
    NativeToolEntry {
        name: "write_file",
        def: native_write_file_tool,
        enabled: |native, _, _| native.write_file,
        parallel_safe: false,
        bypasses_approval: false,
        read_only: false,
        call: NativeToolCall::BlockingMutation(crate::native_tools::write_file),
    },
    NativeToolEntry {
        name: "create_dir",
        def: native_create_dir_tool,
        enabled: |native, _, _| native.write_file,
        parallel_safe: false,
        bypasses_approval: false,
        read_only: false,
        // Intentionally synchronous (not spawn_blocking), matching the
        // legacy dispatch: directory creation has no path-lock waits.
        call: NativeToolCall::SyncText(crate::native_tools::create_dir),
    },
    NativeToolEntry {
        name: "delete_path",
        def: native_delete_path_tool,
        enabled: |native, _, _| native.write_file,
        parallel_safe: false,
        bypasses_approval: false,
        read_only: false,
        call: NativeToolCall::BlockingText(crate::native_tools::delete_path),
    },
    NativeToolEntry {
        name: "move_path",
        def: native_move_path_tool,
        enabled: |native, _, _| native.write_file,
        parallel_safe: false,
        bypasses_approval: false,
        read_only: false,
        call: NativeToolCall::BlockingText(crate::native_tools::move_path),
    },
    NativeToolEntry {
        name: "copy_path",
        def: native_copy_path_tool,
        enabled: |native, _, _| native.write_file,
        parallel_safe: false,
        bypasses_approval: false,
        read_only: false,
        call: NativeToolCall::BlockingText(crate::native_tools::copy_path),
    },
    NativeToolEntry {
        name: "edit_file",
        def: native_edit_file_tool,
        enabled: |native, _, _| native.edit_file,
        parallel_safe: false,
        bypasses_approval: false,
        read_only: false,
        call: NativeToolCall::BlockingMutation(crate::native_tools::edit_file),
    },
    NativeToolEntry {
        name: "run_command",
        def: native_run_command_tool,
        enabled: |native, _, _| native.run_command,
        parallel_safe: false,
        bypasses_approval: false,
        read_only: false,
        call: NativeToolCall::Async(call_run_command),
    },
    NativeToolEntry {
        name: "run_python",
        def: native_run_python_tool,
        enabled: |native, _, _| native.run_python,
        parallel_safe: false,
        bypasses_approval: false,
        read_only: false,
        call: NativeToolCall::Async(call_run_python),
    },
    NativeToolEntry {
        name: "memory_read",
        def: native_memory_read_tool,
        enabled: |_, _, memory_enabled| memory_enabled,
        parallel_safe: false,
        bypasses_approval: true,
        read_only: true,
        call: NativeToolCall::Async(call_memory_read),
    },
    NativeToolEntry {
        name: "memory_modify",
        def: native_memory_modify_tool,
        enabled: |_, _, memory_enabled| memory_enabled,
        parallel_safe: false,
        bypasses_approval: true,
        read_only: false,
        call: NativeToolCall::Async(call_memory_modify),
    },
    // Conversation-level tools below are appended in chat/commands.rs and
    // never exposed via list_native_builtin_tool_defs (enabled = false).
    NativeToolEntry {
        name: crate::chat::todo::TODO_WRITE_TOOL_NAME,
        def: crate::chat::todo::todo_write_tool,
        enabled: |_, _, _| false,
        parallel_safe: false,
        bypasses_approval: true,
        read_only: false,
        call: NativeToolCall::Conversation(crate::chat::todo::handle_conversation_tool_call),
    },
    NativeToolEntry {
        name: crate::chat::todo::TODO_UPDATE_TOOL_NAME,
        def: crate::chat::todo::todo_update_tool,
        enabled: |_, _, _| false,
        parallel_safe: false,
        bypasses_approval: true,
        read_only: false,
        call: NativeToolCall::Conversation(crate::chat::todo::handle_conversation_tool_call),
    },
    NativeToolEntry {
        name: crate::chat::ask_user::ASK_USER_TOOL_NAME,
        def: crate::chat::ask_user::ask_user_tool,
        enabled: |_, _, _| false,
        parallel_safe: false, // spec: ask_user is forced serial with batch flush
        bypasses_approval: true,
        read_only: false,
        call: NativeToolCall::HostMediated,
    },
    // Sub-agent management tools (P3). Appended in chat/commands.rs
    // (`append_agent_subagent_tools`) when the multi-agent toggle is on, so
    // enabled = false here. bypasses_approval = true: spawning/inspecting
    // sub-agents is governed by depth + concurrency caps, not per-call
    // approval prompts.
    NativeToolEntry {
        name: crate::chat::sub_agent::AGENT_TOOL_NAME,
        def: crate::chat::sub_agent::agent_tool,
        enabled: |_, _, _| false,
        // parallel_safe = true: each `agent` spawn runs in isolation (its own
        // synthetic conversation/generation/message history), bypasses approval,
        // and is capped by the SubAgentManager `Semaphore(3)`. Concurrent fan-out
        // is the core value of multi-agent, so a single round may dispatch
        // several spawns in parallel (scheduler caps at 4, semaphore at 3).
        parallel_safe: true,
        bypasses_approval: true,
        read_only: false,
        call: NativeToolCall::SubAgent(crate::chat::sub_agent::dispatch_agent_spawn),
    },
    NativeToolEntry {
        name: crate::chat::sub_agent::CHECK_AGENT_RESULT_TOOL_NAME,
        def: crate::chat::sub_agent::check_agent_result_tool,
        enabled: |_, _, _| false,
        parallel_safe: false,
        bypasses_approval: true,
        read_only: true,
        call: NativeToolCall::SubAgent(crate::chat::sub_agent::dispatch_check_agent_result),
    },
    NativeToolEntry {
        name: crate::chat::sub_agent::LIST_AGENT_TASKS_TOOL_NAME,
        def: crate::chat::sub_agent::list_agent_tasks_tool,
        enabled: |_, _, _| false,
        parallel_safe: false,
        bypasses_approval: true,
        read_only: true,
        call: NativeToolCall::SubAgent(crate::chat::sub_agent::dispatch_list_agent_tasks),
    },
];

pub fn find_entry(name: &str) -> Option<&'static NativeToolEntry> {
    NATIVE_TOOLS.iter().find(|entry| entry.name == name)
}

pub(super) fn text_tool_result(content: String) -> McpToolCallResult {
    McpToolCallResult {
        content,
        is_error: false,
        raw: Value::Null,
        artifacts: Vec::new(),
        structured_content: None,
    }
}

fn call_read_file(
    workspace: &NativeToolWorkspace,
    arguments: &Value,
) -> Result<McpToolCallResult, String> {
    let result = crate::native_tools::read_file(workspace, arguments)?;
    super::registry::read_file_tool_result(result)
}

fn call_web_search(ctx: NativeCallCtx<'_>) -> NativeToolFuture<'_> {
    Box::pin(async move {
        let query = ctx
            .arguments
            .get("query")
            .and_then(|query| query.as_str())
            .unwrap_or_default()
            .trim()
            .to_string();
        if query.is_empty() {
            return Err("web_search query is empty".to_string());
        }
        let retry_attempts = if ctx.settings.retry_enabled {
            ctx.settings.retry_attempts as usize
        } else {
            1
        };
        let results = crate::web_search::search_web(
            ctx.state,
            &ctx.settings.lens.web_search,
            &query,
            retry_attempts,
        )
        .await?;
        let raw = serde_json::to_value(&results).unwrap_or(Value::Null);
        Ok(McpToolCallResult {
            content: crate::web_search::format_web_context(&results),
            is_error: false,
            raw,
            artifacts: Vec::new(),
            structured_content: None,
        })
    })
}

fn call_web_fetch(ctx: NativeCallCtx<'_>) -> NativeToolFuture<'_> {
    Box::pin(async move {
        let content = crate::native_tools::web_fetch(&ctx.state.http, ctx.arguments).await?;
        Ok(text_tool_result(content))
    })
}

fn call_memory_read(ctx: NativeCallCtx<'_>) -> NativeToolFuture<'_> {
    Box::pin(async move {
        // Runtime second gate kept on purpose: the tool list is cached, so a
        // user can disable memory between listing and calling.
        if !ctx.settings.chat_memory.enabled {
            return Err("Chat memory is disabled in Settings".to_string());
        }
        crate::chat::memory::tool_read(ctx.app, ctx.arguments)
    })
}

fn call_memory_modify(ctx: NativeCallCtx<'_>) -> NativeToolFuture<'_> {
    Box::pin(async move {
        if !ctx.settings.chat_memory.enabled {
            return Err("Chat memory is disabled in Settings".to_string());
        }
        crate::chat::memory::tool_modify(ctx.app, ctx.arguments)
    })
}

fn call_run_command(ctx: NativeCallCtx<'_>) -> NativeToolFuture<'_> {
    Box::pin(async move {
        let content = crate::native_tools::run_command(
            ctx.workspace,
            ctx.settings.chat_tools.tool_timeout_ms,
            ctx.arguments,
        )
        .await?;
        Ok(text_tool_result(content))
    })
}

fn call_run_python(ctx: NativeCallCtx<'_>) -> NativeToolFuture<'_> {
    Box::pin(async move {
        super::registry::run_python_via_pyodide(
            ctx.app,
            ctx.state,
            ctx.settings,
            ctx.workspace,
            ctx.arguments,
            ctx.native_ctx.cloned(),
        )
        .await
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXPECTED_ORDER: &[&str] = &[
        "web_search",
        "web_fetch",
        "read_file",
        "list_dir",
        "search_files",
        "glob_files",
        "stat_path",
        "write_file",
        "create_dir",
        "delete_path",
        "move_path",
        "copy_path",
        "edit_file",
        "run_command",
        "run_python",
        "memory_read",
        "memory_modify",
        "todo_write",
        "todo_update",
        "ask_user",
        "agent",
        "check_agent_result",
        "list_agent_tasks",
    ];

    #[test]
    fn registry_order_and_names_match_legacy_exposure_order() {
        let names: Vec<&str> = NATIVE_TOOLS.iter().map(|entry| entry.name).collect();
        assert_eq!(names, EXPECTED_ORDER);
    }

    #[test]
    fn registry_defs_match_entry_names() {
        for entry in NATIVE_TOOLS {
            let def = (entry.def)();
            assert_eq!(def.name, entry.name, "def() name must equal entry name");
            assert_eq!(def.source, "native");
        }
    }

    #[test]
    fn parallel_safe_set_is_exactly_the_narrow_read_whitelist() {
        let parallel: Vec<&str> = NATIVE_TOOLS
            .iter()
            .filter(|entry| entry.parallel_safe)
            .map(|entry| entry.name)
            .collect();
        assert_eq!(
            parallel,
            [
                "web_search",
                "web_fetch",
                "read_file",
                "list_dir",
                "search_files",
                "glob_files",
                "stat_path",
                "agent",
            ],
            "parallel-safe set is intentionally narrow per agent-runtime spec; \
             `agent` joins it because each spawn runs in isolation (own \
             conversation/generation/message history), bypasses approval, and is \
             Semaphore(3)-capped, making concurrent fan-out the core multi-agent value"
        );
    }

    #[test]
    fn approval_bypass_set_matches_legacy_list() {
        let bypass: Vec<&str> = NATIVE_TOOLS
            .iter()
            .filter(|entry| entry.bypasses_approval)
            .map(|entry| entry.name)
            .collect();
        assert_eq!(
            bypass,
            [
                "memory_read",
                "memory_modify",
                "todo_write",
                "todo_update",
                "ask_user",
                "agent",
                "check_agent_result",
                "list_agent_tasks",
            ]
        );
    }

    #[test]
    fn read_only_set_matches_legacy_is_read_only_tool_arm() {
        let read_only: Vec<&str> = NATIVE_TOOLS
            .iter()
            .filter(|entry| entry.read_only)
            .map(|entry| entry.name)
            .collect();
        assert_eq!(
            read_only,
            [
                "web_search",
                "web_fetch",
                "read_file",
                "list_dir",
                "search_files",
                "glob_files",
                "stat_path",
                "memory_read",
                "check_agent_result",
                "list_agent_tasks",
            ],
            "memory_read is read-only but deliberately not parallel-safe"
        );
    }

    #[test]
    fn conversation_tools_are_never_listed_via_builtin_exposure() {
        let native = crate::settings::ChatNativeToolsConfig {
            web_search: true,
            web_fetch: true,
            skill_runtime: true,
            read_file: true,
            write_file: true,
            edit_file: true,
            run_command: true,
            run_python: true,
            workspace_roots: Vec::new(),
        };
        for entry in NATIVE_TOOLS {
            if matches!(
                entry.call,
                NativeToolCall::Conversation(_)
                    | NativeToolCall::HostMediated
                    | NativeToolCall::SubAgent(_)
            ) {
                assert!(
                    !(entry.enabled)(&native, true, true),
                    "{} must be appended via chat/commands.rs, not listed here",
                    entry.name
                );
            }
        }
    }

    /// Exposure-surface snapshot: for fixed settings combinations, the exact
    /// ordered tool-name list returned by `list_native_builtin_tool_defs`
    /// must stay frozen. This is the primary regression guard for the
    /// registry refactor.
    #[test]
    fn builtin_exposure_snapshot_per_settings_combination() {
        use crate::mcp::types::list_native_builtin_tool_defs;
        use crate::settings::ChatNativeToolsConfig;

        fn names(
            native: &ChatNativeToolsConfig,
            web_search_configured: bool,
            memory_enabled: bool,
        ) -> Vec<String> {
            list_native_builtin_tool_defs(native, web_search_configured, memory_enabled)
                .into_iter()
                .map(|tool| tool.name)
                .collect()
        }

        let off = ChatNativeToolsConfig {
            web_search: false,
            web_fetch: false,
            skill_runtime: false,
            read_file: false,
            write_file: false,
            edit_file: false,
            run_command: false,
            run_python: false,
            workspace_roots: Vec::new(),
        };
        assert!(names(&off, true, false).is_empty());

        // web_search requires both the toggle and a configured provider key.
        let mut search_only = off.clone();
        search_only.web_search = true;
        assert!(names(&search_only, false, false).is_empty());
        assert_eq!(names(&search_only, true, false), ["web_search"]);

        // read_file gate exposes the whole read-side group, in order.
        let mut read_only = off.clone();
        read_only.read_file = true;
        assert_eq!(
            names(&read_only, false, false),
            [
                "read_file",
                "list_dir",
                "search_files",
                "glob_files",
                "stat_path"
            ]
        );

        // write gate exposes exactly the whole-file and path tools.
        let mut write_only = off.clone();
        write_only.write_file = true;
        assert_eq!(
            names(&write_only, false, false),
            [
                "write_file",
                "create_dir",
                "delete_path",
                "move_path",
                "copy_path"
            ]
        );

        // memory gate is independent of native toggles.
        assert_eq!(names(&off, false, true), ["memory_read", "memory_modify"]);

        // Everything on: full ordered surface.
        let all = ChatNativeToolsConfig {
            web_search: true,
            web_fetch: true,
            skill_runtime: true,
            read_file: true,
            write_file: true,
            edit_file: true,
            run_command: true,
            run_python: true,
            workspace_roots: Vec::new(),
        };
        assert_eq!(
            names(&all, true, true),
            [
                "web_search",
                "web_fetch",
                "read_file",
                "list_dir",
                "search_files",
                "glob_files",
                "stat_path",
                "write_file",
                "create_dir",
                "delete_path",
                "move_path",
                "copy_path",
                "edit_file",
                "run_command",
                "run_python",
                "memory_read",
                "memory_modify",
            ]
        );
    }
}
